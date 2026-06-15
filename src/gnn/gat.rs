use crate::gnn::activation::Activation;
use crate::gnn::backward::{act_deriv_mul, BackwardGraphLayer, GraphLayer};
use crate::gnn::dropout::Dropout;
use crate::gnn::graph::Graph;
use crate::gnn::layer::{Backward, Layer, LinearLayer};
use crate::gnn::norm::LayerNorm;
use crate::gnn::tensor::Tensor;
use crate::gnn::GnnError;

pub struct GATLayer {
	pub heads: usize,
	pub head_dim: usize,
	pub concat: bool,
	pub leaky_slope: f64,
	pub w: Vec<LinearLayer>, // K linear transforms
	pub a: Vec<Tensor>,      // K attention vectors 1×(2*head_dim)
	pub norm: Option<LayerNorm>,
	pub drop: Option<Dropout>,
	pub act: Option<Activation>,
	last_features: Option<Tensor>,
	last_wh: Vec<Tensor>,
	last_alpha: Vec<Vec<Vec<f64>>>, // [K][N][|nbrs|]
	// Per-head leaky-relu sign cache for the backward pass. Private: external
	// mutation would silently corrupt gradient computation in try_backward_graph.
	last_pre_leaky_pos: Vec<Vec<Vec<bool>>>,
	last_nbr_idxs: Vec<Vec<usize>>,
	last_pre_act: Option<Tensor>,
	d_a: Vec<Tensor>,
}

impl GATLayer {
	pub fn new(
		in_features: usize,
		head_dim: usize,
		heads: usize,
		concat: bool,
		act: Option<Activation>,
		norm: bool,
		drop_rate: f64,
	) -> Self {
		let mut rng = rand::rng();
		Self::with_rng(
			in_features,
			head_dim,
			heads,
			concat,
			act,
			norm,
			drop_rate,
			&mut rng,
		)
	}

	/// Construct a `GATLayer` with deterministic weight init from a seeded RNG.
	/// Use this in tests asserting on training dynamics so runs are reproducible
	/// regardless of system entropy (mirrors [`GCNLayer::with_rng`]).
	#[allow(clippy::too_many_arguments)]
	pub fn with_rng<R: rand::Rng>(
		in_features: usize,
		head_dim: usize,
		heads: usize,
		concat: bool,
		act: Option<Activation>,
		norm: bool,
		drop_rate: f64,
		rng: &mut R,
	) -> Self {
		let scale = (2.0 / (2 * head_dim) as f64).sqrt();
		let w: Vec<_> = (0..heads)
			.map(|_| LinearLayer::with_rng(in_features, head_dim, rng))
			.collect();
		let a: Vec<_> = (0..heads)
			.map(|_| Tensor::rand_with(1, 2 * head_dim, scale, rng))
			.collect();
		let out_dim = if concat { heads * head_dim } else { head_dim };
		Self {
			heads,
			head_dim,
			concat,
			leaky_slope: 0.2,
			w,
			a,
			norm: if norm {
				Some(LayerNorm::new(out_dim))
			} else {
				None
			},
			drop: if drop_rate > 0.0 {
				Some(Dropout::new(drop_rate))
			} else {
				None
			},
			act,
			last_features: None,
			last_wh: Vec::new(),
			last_alpha: Vec::new(),
			last_pre_leaky_pos: Vec::new(),
			last_nbr_idxs: Vec::new(),
			last_pre_act: None,
			d_a: Vec::new(),
		}
	}

	/// Attention weights α for one node over its neighbours: the GAT additive
	/// score e_ij = aᵀ[Wh_i ‖ Wh_j], leaky-relu'd then softmax-normalised
	/// (max-shifted for numerical stability). Returns the per-neighbour weights
	/// and the pre-leaky sign mask, which the backward pass caches to route the
	/// leaky-relu derivative.
	fn node_attention(
		&self,
		wh: &Tensor,
		i: usize,
		nbrs: &[usize],
		a_src: &[f64],
		a_dst: &[f64],
	) -> (Vec<f64>, Vec<bool>) {
		let hd = self.head_dim;
		let mut scores = vec![0.0; nbrs.len()];
		let mut pre_pos = vec![false; nbrs.len()];
		let mut max_score = f64::NEG_INFINITY;
		for (ni, &j) in nbrs.iter().enumerate() {
			let mut e = 0.0;
			for d in 0..hd {
				e += a_src[d] * wh.at(i, d);
				e += a_dst[d] * wh.at(j, d);
			}
			pre_pos[ni] = e >= 0.0;
			if e < 0.0 {
				e *= self.leaky_slope;
			}
			scores[ni] = e;
			if e > max_score {
				max_score = e;
			}
		}
		let mut sum_exp = 0.0;
		for s in &mut scores {
			*s = (*s - max_score).exp();
			sum_exp += *s;
		}
		let alpha_i: Vec<f64> = scores.iter().map(|s| s / sum_exp).collect();
		(alpha_i, pre_pos)
	}

	/// One attention head over the whole graph: for every node compute its
	/// neighbour attention weights and aggregate the transformed neighbour
	/// features Wh into `head_out`. Returns that output plus the cached α and
	/// pre-leaky sign masks (`[N][|nbrs|]`) the backward pass needs.
	fn aggregate_head(
		&self,
		wh: &Tensor,
		nbr_idxs: &[Vec<usize>],
		k: usize,
		n: usize,
	) -> (Tensor, Vec<Vec<f64>>, Vec<Vec<bool>>) {
		let hd = self.head_dim;
		let a_src = &self.a[k].data[..hd];
		let a_dst = &self.a[k].data[hd..2 * hd];

		let mut alpha_k: Vec<Vec<f64>> = vec![Vec::new(); n];
		let mut pre_leaky_pos_k: Vec<Vec<bool>> = vec![Vec::new(); n];
		let mut head_out = Tensor::zeros(n, hd);

		for i in 0..n {
			let nbrs = &nbr_idxs[i];
			if nbrs.is_empty() {
				continue;
			}
			let (alpha_i, pre_pos) = self.node_attention(wh, i, nbrs, a_src, a_dst);
			for (ni, &j) in nbrs.iter().enumerate() {
				let w_ni = alpha_i[ni];
				for d in 0..hd {
					head_out.data[i * hd + d] += w_ni * wh.at(j, d);
				}
			}
			alpha_k[i] = alpha_i;
			pre_leaky_pos_k[i] = pre_pos;
		}
		(head_out, alpha_k, pre_leaky_pos_k)
	}

	/// Scatter a head's output into the layer output: concat heads side-by-side
	/// at the head's column offset, or accumulate into a shared block when heads
	/// are averaged (the mean scale is applied once, after all heads).
	fn write_head_output(&self, output: &mut Tensor, head_out: &Tensor, k: usize, n: usize) {
		let hd = self.head_dim;
		if self.concat {
			let offset = k * hd;
			for i in 0..n {
				for d in 0..hd {
					output.set(i, offset + d, head_out.at(i, d));
				}
			}
		} else {
			for (i, v) in output.data.iter_mut().enumerate() {
				*v += head_out.data[i];
			}
		}
	}
}

impl GraphLayer for GATLayer {
	fn forward_graph(&mut self, g: &Graph, features: &Tensor) -> Tensor {
		let n = g.num_nodes();
		self.last_features = Some(features.clone());

		let mut nbr_idxs = vec![Vec::new(); n];
		for (i, node) in g.nodes.iter().enumerate() {
			let nbrs = g.in_neighbors(&node.id);
			nbr_idxs[i] = nbrs.iter().filter_map(|nbr| g.node_index(nbr)).collect();
		}
		self.last_nbr_idxs = nbr_idxs.clone();

		let out_dim = if self.concat {
			self.heads * self.head_dim
		} else {
			self.head_dim
		};
		let mut output = Tensor::zeros(n, out_dim);

		self.last_wh = Vec::with_capacity(self.heads);
		self.last_alpha = Vec::with_capacity(self.heads);
		self.last_pre_leaky_pos = Vec::with_capacity(self.heads);

		for k in 0..self.heads {
			let wh = self.w[k].forward(features);
			let (head_out, alpha_k, pre_leaky_pos_k) = self.aggregate_head(&wh, &nbr_idxs, k, n);
			self.write_head_output(&mut output, &head_out, k, n);
			self.last_wh.push(wh);
			self.last_alpha.push(alpha_k);
			self.last_pre_leaky_pos.push(pre_leaky_pos_k);
		}

		if !self.concat && self.heads > 1 {
			let scale = 1.0 / self.heads as f64;
			output.scale_inplace(scale);
		}

		if let Some(ref mut nm) = self.norm {
			output = nm.forward(&output);
		}
		self.last_pre_act = Some(output.clone());
		if let Some(a) = self.act {
			output = output.apply(|x| a.forward(x));
		}
		if let Some(ref mut d) = self.drop {
			output = d.forward(&output);
		}
		output
	}

	fn parameters(&self) -> Vec<&Tensor> {
		let mut p = Vec::new();
		for k in 0..self.heads {
			p.extend(self.w[k].parameters());
			p.push(&self.a[k]);
		}
		if let Some(ref nm) = self.norm {
			p.extend(Layer::parameters(nm));
		}
		p
	}

	fn parameters_mut(&mut self) -> Vec<&mut Tensor> {
		let mut p = Vec::new();
		for w in self.w.iter_mut() {
			p.extend(w.parameters_mut());
		}
		for a in self.a.iter_mut() {
			p.push(a);
		}
		if let Some(ref mut nm) = self.norm {
			p.extend(Layer::parameters_mut(nm));
		}
		p
	}

	fn set_training(&mut self, training: bool) {
		if let Some(ref mut d) = self.drop {
			d.set_training(training);
		}
	}
}

impl GATLayer {
	/// Fallible backward pass. Returns [`GnnError::MissingForwardState`] when
	/// invoked before a successful `forward_graph` (or after state was reset),
	/// instead of panicking. Prefer this over the [`BackwardGraphLayer`] trait
	/// method when callers want to recover from invariant breaches (e.g. the
	/// autonomic GNN paths in the kern daemon).
	pub fn try_backward_graph(&mut self, g: &Graph, d_out: &Tensor) -> Result<Tensor, GnnError> {
		if self.d_a.is_empty() {
			self.zero_grads();
		}
		let n = g.num_nodes();
		let mut grad = d_out.clone();

		if let Some(ref d) = self.drop {
			grad = d.backward(&grad);
		}
		if let Some(a) = self.act {
			let pre_act = self
				.last_pre_act
				.as_ref()
				.ok_or(GnnError::MissingForwardState("gat::last_pre_act"))?;
			grad = act_deriv_mul(a, &grad, pre_act);
		}
		if let Some(ref mut nm) = self.norm {
			grad = nm.backward(&grad);
		}

		let in_f = self
			.last_features
			.as_ref()
			.ok_or(GnnError::MissingForwardState("gat::last_features"))?
			.cols;
		let mut d_features = Tensor::zeros(n, in_f);
		let hd = self.head_dim;

		for k in 0..self.heads {
			let d_head = if self.concat {
				let offset = k * hd;
				let mut dh = Tensor::zeros(n, hd);
				for i in 0..n {
					for d in 0..hd {
						dh.data[i * hd + d] = grad.at(i, offset + d);
					}
				}
				dh
			} else {
				grad.scale(1.0 / self.heads as f64)
			};

			let mut d_wh = Tensor::zeros(n, hd);

			for i in 0..n {
				let nbrs = &self.last_nbr_idxs[i];
				if nbrs.is_empty() {
					continue;
				}
				let alpha_i = &self.last_alpha[k][i];
				let pre_pos_i = &self.last_pre_leaky_pos[k][i];

				for (ni, &j) in nbrs.iter().enumerate() {
					for d in 0..hd {
						d_wh.data[j * hd + d] += alpha_i[ni] * d_head.at(i, d);
					}
				}

				let mut d_alpha = vec![0.0; nbrs.len()];
				for (ni, &j) in nbrs.iter().enumerate() {
					for d in 0..hd {
						d_alpha[ni] += d_head.at(i, d) * self.last_wh[k].at(j, d);
					}
				}

				let sum_ad: f64 = alpha_i.iter().zip(&d_alpha).map(|(a, da)| a * da).sum();
				let d_e: Vec<f64> = alpha_i
					.iter()
					.zip(&d_alpha)
					.map(|(a, da)| a * (da - sum_ad))
					.collect();

				let d_pl: Vec<f64> = d_e
					.iter()
					.zip(pre_pos_i)
					.map(|(de, &pos)| if pos { *de } else { *de * self.leaky_slope })
					.collect();

				for (ni, &j) in nbrs.iter().enumerate() {
					for d in 0..hd {
						self.d_a[k].data[d] += d_pl[ni] * self.last_wh[k].at(i, d);
						self.d_a[k].data[hd + d] += d_pl[ni] * self.last_wh[k].at(j, d);
						d_wh.data[i * hd + d] += d_pl[ni] * self.a[k].data[d];
						d_wh.data[j * hd + d] += d_pl[ni] * self.a[k].data[hd + d];
					}
				}
			}

			let d_feat_k = self.w[k].backward(&d_wh);
			d_features.add_inplace(&d_feat_k)?;
		}
		Ok(d_features)
	}
}

impl BackwardGraphLayer for GATLayer {
	/// Trait-level backward pass.
	///
	/// Delegates to [`GATLayer::try_backward_graph`]. On error (e.g. backward
	/// invoked before forward), logs via `tracing::error!` and returns a
	/// zero-shaped gradient instead of panicking — this keeps the daemon
	/// alive when an autonomic path mis-orders calls. Callers that need to
	/// observe the error should call `try_backward_graph` directly.
	fn backward_graph(&mut self, g: &Graph, d_out: &Tensor) -> Tensor {
		match self.try_backward_graph(g, d_out) {
			Ok(t) => t,
			Err(e) => {
				tracing::error!(error = %e, "GAT backward_graph failed; returning zero gradient");
				let in_f = self.last_features.as_ref().map(|f| f.cols).unwrap_or(0);
				Tensor::zeros(g.num_nodes(), in_f)
			}
		}
	}

	fn param_grads(&self) -> Vec<&Tensor> {
		let mut g = Vec::new();
		for k in 0..self.heads {
			g.extend(self.w[k].param_grads());
			g.push(&self.d_a[k]);
		}
		if let Some(ref nm) = self.norm {
			g.extend(Backward::param_grads(nm));
		}
		g
	}

	fn param_grads_mut(&mut self) -> Vec<&mut Tensor> {
		let mut g: Vec<&mut Tensor> = Vec::new();
		for w in self.w.iter_mut() {
			g.extend(w.param_grads_mut());
		}
		for da in self.d_a.iter_mut() {
			g.push(da);
		}
		if let Some(ref mut nm) = self.norm {
			g.extend(Backward::param_grads_mut(nm));
		}
		g
	}

	fn zero_grads(&mut self) {
		self.d_a = (0..self.heads)
			.map(|_| Tensor::zeros(1, 2 * self.head_dim))
			.collect();
		for k in 0..self.heads {
			self.w[k].zero_grads();
		}
		if let Some(ref mut nm) = self.norm {
			Backward::zero_grads(nm);
		}
	}
}

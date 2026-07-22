//! Who an *edge* may be shown to.
//!
//! A `Reason` carries no ACL of its own, but `link` writes its body by quoting up
//! to 500 chars of BOTH endpoint texts (`explain_relationship_prompt`,
//! `src/base/util.rs`), so the text belongs to the endpoints. Every surface that
//! renders an entity renders its incident edges, so every one of them has to
//! re-derive the same verdict — and the copies are what drift. It lives here once.

use crate::base::graph::GraphGnn;
use crate::base::search::find_entity;
use crate::base::types::{Entity, Reason};

/// What an edge endpoint's ACL says about serving the edge that quotes it.
///
/// Three outcomes and not two, because `find_entity` (`src/base/search.rs`)
/// searches only the **resident** kern map — `loaded` is `kerns.get` and `all()`
/// is `kerns.values()`, neither of which sees `unloaded` or the cold tier. So
/// "did not resolve" is emphatically not "does not exist", and treating the two
/// alike is the fail-open case: a scoped row that a GC cold-spill
/// (`src/tick/stigmergy.rs`) or a kern-cap unload (`GraphGnn::unload`) made
/// non-resident is *still alive in the store with its ACL intact* and reads back
/// here as absent. The edge quoting it survives because a kern hosts a reason iff
/// it hosts its `from` (`src/base/reason.rs`) — `move_entity` leaves an incoming
/// edge in the *source* kern, and `remove_entity` cascades only within one kern,
/// so nothing ever sweeps it.
pub(crate) enum Endpoint {
	/// Resolved, and the caller's admission test cleared it.
	Admitted,
	/// Resolved, and the caller's admission test refused it.
	Withheld,
	/// Did not resolve. Could be a genuinely dangling id — ordinary here, `to` is
	/// optional in `add_reason` — or a scoped row we simply cannot see.
	Unresolved,
}

/// `admits` is the *caller's* rule, not a fixed one: the resources surface can
/// name no principal, so for it admission is `Acl::is_public`; the `query` tool
/// takes the caller's `principals` and admission is the ACL half of
/// `matches_filter`. Passing the rule in is what lets one verdict serve both
/// without either surface deciding what the other's "allowed" means.
pub(crate) fn endpoint(g: &GraphGnn, id: &str, admits: &dyn Fn(&Entity) -> bool) -> Endpoint {
	match find_entity(g, id) {
		Some((t, _)) if admits(&t) => Endpoint::Admitted,
		Some(_) => Endpoint::Withheld,
		None => Endpoint::Unresolved,
	}
}

/// Verdict for one edge incident to `owner`: `None` drops it, `Some(false)`
/// renders it with `text` withheld, `Some(true)` renders it whole.
///
/// Redaction rather than a drop on `Unresolved` is what keeps default-deny from
/// becoming deny-all: a dangling endpoint is ordinary, and dropping every edge
/// with one would hide an admitted entity's own structure. The residual is that
/// an unresolved endpoint id is still named — a content hash, so at worst it
/// confirms a guessed text, never discloses one.
pub(crate) fn incident_edge(
	g: &GraphGnn,
	owner: &str,
	re: &Reason,
	admits: &dyn Fn(&Entity) -> bool,
) -> Option<bool> {
	// `collect_reason_ids` returns only incident edges, so the far end is `to`
	// when this entity is the `from` and `from` otherwise.
	let far = if re.from == owner { &re.to } else { &re.from };
	match endpoint(g, far, admits) {
		Endpoint::Withheld => None,
		Endpoint::Unresolved => Some(false),
		Endpoint::Admitted => Some(true),
	}
}

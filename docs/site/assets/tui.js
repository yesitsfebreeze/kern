// Terminal's base.html emits extra_javascript in <head> with no defer, so this
// must wait for the body itself before touching the DOM.
document.addEventListener("DOMContentLoaded", () => {
	const root = document.querySelector(".tui");
	const menuEl = document.getElementById("tui-menu");
	const dataEl = document.getElementById("tui-nav-data");
	const pane = document.getElementById("terminal-mkdocs-main-content");
	if (!root || !menuEl || !dataEl || !pane) return;

	const items = [];
	const walk = (nodes, group) => {
		for (const n of nodes) {
			if (n.children) walk(n.children, n.title);
			else items.push({ title: n.title, url: n.url, group: group || "" });
		}
	};
	walk(JSON.parse(dataEl.textContent), "");
	if (!items.length) return;

	const slug = (u) => {
		const parts = u.replace(/\/$/, "").split("/").filter(Boolean);
		return parts.length ? parts[parts.length - 1] : "index";
	};

	const out = document.getElementById("tui-out");
	const write = (msg) => {
		out.textContent = msg;
	};

	pane.insertAdjacentHTML("beforebegin", '<hr class="tui-rule">');

	let group = null;
	const rows = items.map((item, i) => {
		if (item.group !== group) {
			group = item.group;
			const h = document.createElement("div");
			h.className = "tui-group";
			h.textContent = group.toUpperCase();
			menuEl.appendChild(h);
		}
		const a = document.createElement("a");
		a.className = "tui-row";
		a.href = item.url;
		a.dataset.i = i;
		a.innerHTML =
			'<span class="tui-caret"></span>' +
			'<span class="tui-name"></span>' +
			'<span class="tui-desc"></span>';
		a.querySelector(".tui-name").textContent = slug(item.url);
		a.querySelector(".tui-desc").textContent = item.title;
		menuEl.appendChild(a);
		return a;
	});

	// One cursor walks the whole screen in reading order: the menu rows first,
	// then the open page's headings, blocks, and the links inside each block.
	// Enter acts on whatever the cursor is holding.
	const BLOCKS = "h1, h2, h3, h4, h5, h6, p, li, blockquote, pre, table";
	let stops = [];
	let cursor = 0;

	const buildStops = () => {
		stops = rows.map((el, i) => ({ el, kind: "menu", i }));
		for (const block of pane.querySelectorAll(BLOCKS)) {
			// A list item's own <p> would otherwise stop twice on the same text.
			if (block.parentElement.closest("li, blockquote") && block.tagName === "P") continue;
			const heading = /^H[1-6]$/.test(block.tagName);
			stops.push({ el: block, kind: heading ? "heading" : "block" });
			for (const a of block.querySelectorAll("a[href]")) {
				stops.push({ el: a, kind: "link", url: a.getAttribute("href") });
			}
		}
	};

	const paint = () => {
		for (const s of stops) s.el.classList.remove("is-on");
		const s = stops[cursor];
		if (!s) return;
		s.el.classList.add("is-on");
		s.el.scrollIntoView({ block: "nearest" });
	};

	const visible = (s) => !(s.kind === "menu" && s.el.hidden);

	const step = (dir) => {
		for (let n = 1; n <= stops.length; n++) {
			const i = (cursor + dir * n + stops.length * n) % stops.length;
			if (visible(stops[i])) {
				cursor = i;
				paint();
				return;
			}
		}
	};

	const cache = new Map();
	let seq = 0;

	const load = async (i) => {
		const item = items[i];
		const mine = ++seq;
		if (!cache.has(item.url)) {
			root.classList.add("is-loading");
			try {
				const res = await fetch(item.url, { credentials: "same-origin" });
				if (!res.ok) throw new Error(res.status);
				const doc = new DOMParser().parseFromString(await res.text(), "text/html");
				const main = doc.getElementById("terminal-mkdocs-main-content");
				cache.set(item.url, main ? main.innerHTML : null);
			} catch {
				cache.set(item.url, null);
			}
			root.classList.remove("is-loading");
		}
		if (mine !== seq) return false;
		const html = cache.get(item.url);
		if (html == null) {
			write(`cat: ${slug(item.url)}: cannot read (open it directly)`);
			return false;
		}
		pane.innerHTML = html;
		document.title = item.title + " — " + document.title.split(" — ").pop();
		history.replaceState(null, "", item.url);
		root.dataset.current = item.url;
		buildStops();
		return true;
	};

	// Enter: a menu row loads that page and drops the cursor into it; a link is
	// followed; a heading or paragraph has nothing to open.
	const enter = async () => {
		const s = stops[cursor];
		if (!s) return;
		if (s.kind === "link") {
			window.location.href = s.url;
		} else if (s.kind === "menu") {
			if (await load(s.i)) {
				const first = stops.findIndex((x) => x.kind !== "menu");
				if (first >= 0) {
					cursor = first;
					paint();
				}
			}
		}
	};

	rows.forEach((r, i) => {
		r.addEventListener("click", (e) => {
			if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
			e.preventDefault();
			cursor = i;
			paint();
			enter();
		});
	});

	const input = document.getElementById("tui-input");
	const find = (arg) =>
		items.findIndex(
			(i) => slug(i.url) === arg || i.title.toLowerCase() === arg.toLowerCase(),
		);

	const commands = {
		help: () =>
			"ls [section]    list pages\n" +
			"open <page>     load a page below\n" +
			"cd <page>       same as open\n" +
			"sections        list sections\n" +
			"clear           clear this output\n" +
			"\n↑↓ / jk step through headings, paragraphs and links.\n" +
			"↵ follows a link or opens the selected page.",
		sections: () => [...new Set(items.map((i) => i.group))].filter(Boolean).join("\n"),
		ls: (arg) => {
			const list = arg
				? items.filter((i) => i.group.toLowerCase() === arg.toLowerCase())
				: items;
			if (!list.length) return `ls: ${arg}: no such section`;
			return list.map((i) => slug(i.url).padEnd(20) + i.title).join("\n");
		},
		open: (arg) => {
			if (!arg) return "open: needs a page name";
			const i = find(arg);
			if (i < 0) return `open: ${arg}: no such page`;
			cursor = i;
			paint();
			enter();
			return "";
		},
		clear: () => "",
	};
	commands.cd = commands.open;

	const applyFilter = (q) => {
		const needle = q.toLowerCase();
		let first = -1;
		rows.forEach((r, i) => {
			const hit =
				!needle ||
				slug(items[i].url).toLowerCase().includes(needle) ||
				items[i].title.toLowerCase().includes(needle);
			r.hidden = !hit;
			if (hit && first < 0) first = i;
		});
		menuEl.querySelectorAll(".tui-group").forEach((g) => {
			let n = g.nextElementSibling;
			let any = false;
			while (n && !n.classList.contains("tui-group")) {
				if (!n.hidden) any = true;
				n = n.nextElementSibling;
			}
			g.hidden = !any;
		});
		if (first >= 0) {
			cursor = first;
			paint();
		}
		write(first < 0 ? `no page matches '${q}'` : "");
	};

	input.addEventListener("input", () => {
		if (input.value.startsWith("/")) applyFilter(input.value.slice(1));
		else if (!input.value) applyFilter("");
	});

	document.getElementById("tui-prompt").addEventListener("submit", (e) => {
		e.preventDefault();
		const raw = input.value.trim();
		if (raw.startsWith("/")) {
			const hit = stops.findIndex((s) => s.kind === "menu" && !s.el.hidden);
			if (hit >= 0) {
				cursor = hit;
				paint();
				enter();
			}
			return;
		}
		input.value = "";
		if (!raw) return;
		const [cmd, ...rest] = raw.split(/\s+/);
		const fn = commands[cmd];
		write(fn ? fn(rest.join(" ")) : `${cmd}: command not found — try 'help'`);
	});

	document.addEventListener("keydown", (e) => {
		const typing = e.target === input;
		if (typing && !["ArrowUp", "ArrowDown", "Escape"].includes(e.key)) return;
		if (e.key === "ArrowDown" || (e.key === "j" && !typing)) {
			e.preventDefault();
			step(1);
		} else if (e.key === "ArrowUp" || (e.key === "k" && !typing)) {
			e.preventDefault();
			step(-1);
		} else if (e.key === "Enter" && !typing) {
			e.preventDefault();
			enter();
		} else if (e.key === "Escape") {
			input.blur();
		} else if (e.key === "/" && !typing) {
			e.preventDefault();
			input.value = "/";
			input.focus();
		}
	});

	buildStops();
	// Start on the row for the page the server already rendered.
	const here = items.findIndex((i) => i.url === root.dataset.current);
	cursor = Math.max(0, here);
	paint();
});

/* ===========================================================================
 * Pinterest board -> zip, straight from the browser console.
 *
 * No login automation, no dependencies. You're already logged in in your
 * browser, so this just reads what the page can see.
 *
 * HOW TO USE
 *   1. Open your board in the browser:  https://www.pinterest.com/<you>/<board>/
 *   2. Open DevTools (F12) -> Console tab.
 *   3. Paste this entire file, press Enter.
 *   4. It auto-scrolls to load every pin, fetches each image, and downloads
 *      a single  pinterest-board.zip.
 *
 * Notes
 *   - Pinterest virtualises the grid (off-screen pins are removed from the
 *     DOM), so the script collects image URLs while it scrolls.
 *   - The zip is built in pure JS (store / no compression -- JPEGs are
 *     already compressed), so the page's CSP can't block a CDN import.
 *   - If a few images fail, it's almost always CORS on i.pinimg.com; the
 *     script reports the count and zips the rest.
 * ========================================================================= */

(async () => {
	// --- tiny store-only ZIP writer ------------------------------------------
	const crcTable = (() => {
		const t = new Uint32Array(256);
		for (let n = 0; n < 256; n++) {
			let c = n;
			for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
			t[n] = c >>> 0;
		}
		return t;
	})();
	const crc32 = (u8) => {
		let c = 0xffffffff;
		for (let i = 0; i < u8.length; i++) c = crcTable[(c ^ u8[i]) & 0xff] ^ (c >>> 8);
		return (c ^ 0xffffffff) >>> 0;
	};

	function buildZip(files) {
		// files: [{ name, data: Uint8Array }]
		const enc = new TextEncoder();
		const chunks = [];
		const central = [];
		let offset = 0;

		const u16 = (n) => new Uint8Array([n & 0xff, (n >>> 8) & 0xff]);
		const u32 = (n) =>
			new Uint8Array([n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff]);

		for (const f of files) {
			const name = enc.encode(f.name);
			const crc = crc32(f.data);
			const size = f.data.length;

			const local = concat([
				u32(0x04034b50), u16(20), u16(0), u16(0), u16(0), u16(0),
				u32(crc), u32(size), u32(size), u16(name.length), u16(0),
				name, f.data,
			]);
			chunks.push(local);

			central.push(concat([
				u32(0x02014b50), u16(20), u16(20), u16(0), u16(0), u16(0), u16(0),
				u32(crc), u32(size), u32(size), u16(name.length), u16(0), u16(0),
				u16(0), u16(0), u32(0), u32(offset), name,
			]));
			offset += local.length;
		}

		const cd = concat(central);
		const eocd = concat([
			u32(0x06054b50), u16(0), u16(0), u16(files.length), u16(files.length),
			u32(cd.length), u32(offset), u16(0),
		]);
		return new Blob([concat(chunks), cd, eocd], { type: 'application/zip' });
	}

	function concat(parts) {
		let len = 0;
		for (const p of parts) len += p.length;
		const out = new Uint8Array(len);
		let o = 0;
		for (const p of parts) { out.set(p, o); o += p.length; }
		return out;
	}

	// --- collect pin images while scrolling ----------------------------------
	const toOriginal = (url) => url.replace(/\/(\d+x|\d+x\d+)\//, '/originals/');
	const pins = new Map(); // id -> { id, title, src }

	function collect() {
		for (const a of document.querySelectorAll('a[href*="/pin/"]')) {
			const m = a.getAttribute('href').match(/\/pin\/([^/]+)\//);
			const img = a.querySelector('img');
			if (!m || !img) continue;
			const src = img.currentSrc || img.src;
			if (src && !pins.has(m[1])) {
				pins.set(m[1], { id: m[1], title: (img.alt || '').trim(), src });
			}
		}
	}

	console.log('%cScrolling board to load all pins...', 'color:#a3e635');
	let stable = 0, last = 0;
	for (let step = 0; step < 100 && stable < 5; step++) {
		collect();
		stable = pins.size === last ? stable + 1 : 0;
		last = pins.size;
		console.log(`  pins found: ${pins.size}`);
		window.scrollBy(0, window.innerHeight * 0.9);
		await new Promise((r) => setTimeout(r, 800));
	}
	collect();

	if (pins.size === 0) {
		console.error('No pins found -- are you on a board page?');
		return;
	}
	console.log(`%cTotal pins: ${pins.size}. Downloading images...`, 'color:#a3e635');

	// --- fetch every image ---------------------------------------------------
	const list = [...pins.values()];
	const files = [];
	let ok = 0, fail = 0;

	for (let i = 0; i < list.length; i++) {
		const pin = list[i];
		const candidates = [toOriginal(pin.src), pin.src.replace(/\/\d+x\//, '/736x/'), pin.src];
		let got = null;
		for (const url of candidates) {
			try {
				const r = await fetch(url, { mode: 'cors' });
				if (!r.ok) continue;
				const buf = new Uint8Array(await r.arrayBuffer());
				if (buf.length < 1024) continue;
				got = buf;
				break;
			} catch { /* try next */ }
		}
		if (got) {
			const ext = (pin.src.split('?')[0].split('.').pop() || 'jpg').slice(0, 4);
			files.push({ name: `${String(i + 1).padStart(2, '0')}_${pin.id}.${ext}`, data: got });
			ok++;
		} else {
			fail++;
		}
		console.log(`  ${ok + fail}/${list.length}  (ok ${ok}, failed ${fail})`);
	}

	if (files.length === 0) {
		console.error('All fetches failed (CORS). Try the credential crawler instead.');
		return;
	}

	// --- download the zip ----------------------------------------------------
	const blob = buildZip(files);
	const a = document.createElement('a');
	a.href = URL.createObjectURL(blob);
	a.download = 'pinterest-board.zip';
	document.body.appendChild(a);
	a.click();
	a.remove();
	setTimeout(() => URL.revokeObjectURL(a.href), 10000);

	console.log(
		`%cDone -- pinterest-board.zip (${ok} images${fail ? `, ${fail} failed` : ''})`,
		'color:#a3e635;font-weight:bold'
	);
})();

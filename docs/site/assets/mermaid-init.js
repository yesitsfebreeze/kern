// Terminal's base.html renders extra_javascript as a plain <script src>, dropping
// type="module" — so bootstrap the ESM build from a module tag we inject ourselves.
const s = document.createElement("script");
s.type = "module";
s.textContent = `
  import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";
  mermaid.initialize({ startOnLoad: true, theme: "dark", securityLevel: "strict" });
`;
document.head.appendChild(s);

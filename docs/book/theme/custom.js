// kern docs — Ctrl+K / Cmd+K opens search. Adds a subtle hint badge.

(function () {
  function isMac() {
    return /Mac|iPod|iPhone|iPad/.test(navigator.platform);
  }

  function bindSearchShortcut() {
    var bar = document.getElementById("searchbar");
    if (!bar) return;

    // Placeholder mirrors the shortcut hint.
    bar.setAttribute(
      "placeholder",
      "Search  (" + (isMac() ? "⌘K" : "Ctrl+K") + ")"
    );

    // Inject a small hint badge adjacent to the input.
    var outer = document.getElementById("searchbar-outer") || bar.parentNode;
    if (outer && !outer.querySelector(".search-hint")) {
      var hint = document.createElement("span");
      hint.className = "search-hint";
      hint.textContent = isMac() ? "⌘K" : "Ctrl+K";
      bar.insertAdjacentElement("afterend", hint);
    }

    document.addEventListener("keydown", function (e) {
      var mod = isMac() ? e.metaKey : e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        // mdBook's search toggle is bound to the magnifier button; trigger via
        // the search icon if the box isn't already open, then focus the input.
        var icon = document.getElementById("search-toggle");
        var wrapper = document.getElementById("search-wrapper");
        var hidden =
          wrapper && wrapper.classList.contains("hidden");
        if (hidden && icon) {
          icon.click();
        }
        setTimeout(function () {
          bar.focus();
          bar.select();
        }, 0);
      } else if (e.key === "Escape") {
        if (document.activeElement === bar) {
          bar.blur();
        }
      }
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", bindSearchShortcut);
  } else {
    bindSearchShortcut();
  }
})();

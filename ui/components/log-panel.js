import { BaseElement } from "./helpers.js";
import { LOG_INFO, LOG_WARN, LOG_ERROR } from "./protocol.js";

const MAX_LINES = 500;
const LEVEL_NAME = { [LOG_INFO]: "INFO", [LOG_WARN]: "WARN", [LOG_ERROR]: "ERR " };
const LEVEL_CLASS = { [LOG_INFO]: "lp-info", [LOG_WARN]: "lp-warn", [LOG_ERROR]: "lp-error" };

/**
 * Fixed bottom panel that streams log messages from the firmware.
 * Default-collapsed; shows only a slim header strip until the user expands
 * it. When new log lines arrive while collapsed, a badge counts them.
 */
export class LogPanel extends BaseElement {
  init() {
    this.unseen = 0;
    this.paused = false;
    this.expanded = false;
    this.userScrolledUp = false;

    this.classList.add("log-panel");

    const header = document.createElement("div");
    header.className = "log-panel-header";

    const chevron = document.createElement("span");
    chevron.className = "log-panel-chevron";
    chevron.textContent = "\u25B2"; // up triangle when collapsed
    header.appendChild(chevron);

    const title = document.createElement("span");
    title.className = "log-panel-title";
    title.textContent = "Logs";
    header.appendChild(title);

    const badge = document.createElement("span");
    badge.className = "log-panel-badge";
    header.appendChild(badge);

    const spacer = document.createElement("span");
    spacer.className = "log-panel-spacer";
    header.appendChild(spacer);

    const pauseBtn = document.createElement("button");
    pauseBtn.className = "btn btn-sm log-panel-btn";
    pauseBtn.type = "button";
    pauseBtn.textContent = "Pause";
    pauseBtn.title = "Pause appending new log lines";
    header.appendChild(pauseBtn);

    const clearBtn = document.createElement("button");
    clearBtn.className = "btn btn-sm log-panel-btn";
    clearBtn.type = "button";
    clearBtn.textContent = "Clear";
    clearBtn.title = "Clear log buffer";
    header.appendChild(clearBtn);

    const body = document.createElement("div");
    body.className = "log-panel-body";

    const list = document.createElement("ol");
    list.className = "log-panel-list";
    body.appendChild(list);

    this.appendChild(header);
    this.appendChild(body);

    this._chevron = chevron;
    this._badge = badge;
    this._pauseBtn = pauseBtn;
    this._body = body;
    this._list = list;

    // Header click toggles expand. Buttons inside the header stop propagation.
    header.addEventListener("click", () => this.toggle());
    pauseBtn.addEventListener("click", (e) => { e.stopPropagation(); this.togglePause(); });
    clearBtn.addEventListener("click", (e) => { e.stopPropagation(); this.clear(); });

    // Track whether the user has scrolled away from the bottom; if they
    // have, don't auto-scroll on new lines.
    body.addEventListener("scroll", () => {
      const slack = 4; // px tolerance for "at bottom"
      this.userScrolledUp =
        body.scrollTop + body.clientHeight < body.scrollHeight - slack;
    });

    this._renderBadge();
    this._applyExpanded();
  }

  /**
   * Append a single log entry (called by app.js).
   * @param {number} level numeric level (LOG_INFO|WARN|ERROR)
   * @param {string} msg
   */
  append(level, msg) {
    if (this.paused) return;
    const li = document.createElement("li");
    li.className = "log-panel-line " + (LEVEL_CLASS[level] || "lp-info");
    const lvl = document.createElement("span");
    lvl.className = "log-panel-level";
    lvl.textContent = LEVEL_NAME[level] || "INFO";
    const text = document.createElement("span");
    text.className = "log-panel-msg";
    text.textContent = msg;
    li.appendChild(lvl);
    li.appendChild(text);
    this._list.appendChild(li);

    // Cap line count by removing oldest in batches to keep DOM cheap.
    while (this._list.childElementCount > MAX_LINES) {
      this._list.removeChild(this._list.firstChild);
    }

    if (!this.expanded) {
      this.unseen++;
      this._renderBadge();
    } else if (!this.userScrolledUp) {
      this._body.scrollTop = this._body.scrollHeight;
    }
  }

  clear() {
    this._list.replaceChildren();
    this.unseen = 0;
    this._renderBadge();
  }

  togglePause() {
    this.paused = !this.paused;
    this._pauseBtn.textContent = this.paused ? "Resume" : "Pause";
    this._pauseBtn.classList.toggle("log-panel-paused", this.paused);
  }

  toggle() {
    this.expanded = !this.expanded;
    if (this.expanded) {
      this.unseen = 0;
      this._renderBadge();
    }
    this._applyExpanded();
  }

  _applyExpanded() {
    this.classList.toggle("expanded", this.expanded);
    this._chevron.textContent = this.expanded ? "\u25BC" : "\u25B2";
    if (this.expanded && !this.userScrolledUp) {
      // Defer to next frame so layout has the body visible first.
      requestAnimationFrame(() => {
        this._body.scrollTop = this._body.scrollHeight;
      });
    }
  }

  _renderBadge() {
    if (this.unseen > 0 && !this.expanded) {
      this._badge.textContent = String(this.unseen);
      this._badge.classList.add("visible");
    } else {
      this._badge.textContent = "";
      this._badge.classList.remove("visible");
    }
  }
}

customElements.define("log-panel", LogPanel);

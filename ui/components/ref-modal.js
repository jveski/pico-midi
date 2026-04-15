import { BaseElement } from "./helpers.js";

// Track all ref-modal instances for mutual exclusivity and scroll lock
const allModals = new Set();

// Single document-level Escape handler (registered once)
let escapeListenerRegistered = false;
function registerEscapeListener() {
  if (escapeListenerRegistered) return;
  escapeListenerRegistered = true;
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      for (const modal of allModals) {
        if (modal.isOpen) {
          modal.close();
          break; // Close only the first open modal found
        }
      }
    }
  });
}

export class RefModal extends BaseElement {
  init() {
    const title = this.dataset.title || "Reference";

    // Build modal structure
    const overlay = document.createElement("div");
    overlay.className = "modal-overlay";

    const dialog = document.createElement("div");
    dialog.className = "modal-dialog";

    const header = document.createElement("div");
    header.className = "modal-header";
    header.innerHTML = `<h2>${title}</h2><button class="btn btn-sm modal-close" title="Close">&times;</button>`;

    const body = document.createElement("div");
    body.className = "modal-body";

    // Move existing children into the modal body
    while (this.firstChild) {
      body.appendChild(this.firstChild);
    }

    dialog.appendChild(header);
    dialog.appendChild(body);
    overlay.appendChild(dialog);
    this.appendChild(overlay);

    // Close on overlay click (but not dialog click)
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) this.close();
    });

    // Close button
    header.querySelector(".modal-close").addEventListener("click", () => this.close());

    // Register this modal and set up Escape handler
    allModals.add(this);
    registerEscapeListener();
  }

  open() {
    // Close any other open modal first (mutual exclusivity)
    for (const modal of allModals) {
      if (modal !== this && modal.isOpen) {
        modal.classList.remove("open");
      }
    }
    this.classList.add("open");
    document.body.classList.add("modal-open");
  }

  close() {
    this.classList.remove("open");
    // Only remove body scroll lock if no modals are open
    let anyOpen = false;
    for (const modal of allModals) {
      if (modal.isOpen) { anyOpen = true; break; }
    }
    if (!anyOpen) {
      document.body.classList.remove("modal-open");
    }
  }

  get isOpen() {
    return this.classList.contains("open");
  }

  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }
}

customElements.define("ref-modal", RefModal);

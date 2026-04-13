import { BaseElement } from "./helpers.js";

const MOBILE_BREAKPOINT = 1024;

export class SectionNav extends BaseElement {
  init() {
    this._nav = this.querySelector(".section-nav");
    this._items = this.querySelectorAll(".section-nav-item");
    this._observer = null;
    this._scrollContainer = null;
    this._isMobile = false;

    // Click handler for nav items
    this._nav.addEventListener("click", (e) => {
      e.preventDefault();
      const item = e.target.closest(".section-nav-item");
      if (!item) return;
      const targetId = item.dataset.target;
      const target = document.getElementById(targetId);
      if (target) {
        target.scrollIntoView({ behavior: "smooth", block: "start" });
      }
    });

    // Set up IntersectionObserver once the scroll container is available
    requestAnimationFrame(() => this._setupObserver());

    // Re-create observer on resize if breakpoint changes
    this._resizeHandler = () => {
      const mobile = window.innerWidth <= MOBILE_BREAKPOINT;
      if (mobile !== this._isMobile) {
        this._isMobile = mobile;
        this._teardownObserver();
        this._setupObserver();
      }
    };
    window.addEventListener("resize", this._resizeHandler);
  }

  _getObserverRoot() {
    // On desktop, the scroll container is the config-panel element.
    // On mobile, config-panel has overflow: visible, so use null (viewport).
    this._isMobile = window.innerWidth <= MOBILE_BREAKPOINT;
    if (this._isMobile) return null;
    const panel = this.parentElement.querySelector("config-panel");
    return panel || null;
  }

  _setupObserver() {
    const root = this._getObserverRoot();

    this._observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          const id = entry.target.id;
          this._setActive(id);
        }
      }
    }, {
      root,
      rootMargin: "-10% 0px -80% 0px",
      threshold: 0,
    });

    // Observe all section targets
    this._items.forEach(item => {
      const target = document.getElementById(item.dataset.target);
      if (target) this._observer.observe(target);
    });
  }

  _teardownObserver() {
    if (this._observer) {
      this._observer.disconnect();
      this._observer = null;
    }
  }

  _setActive(targetId) {
    this._items.forEach(item => {
      item.classList.toggle("active", item.dataset.target === targetId);
    });

    // Scroll the active nav item into view if needed
    const activeItem = this._nav.querySelector(".section-nav-item.active");
    if (activeItem) {
      activeItem.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }
}

customElements.define("section-nav", SectionNav);

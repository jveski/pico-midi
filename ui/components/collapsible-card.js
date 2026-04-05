export class CollapsibleCard extends HTMLElement {
  connectedCallback() {
    if (this._init) return;
    this._init = true;

    const section = this.dataset.section;
    const title = this.dataset.title;
    const badgeId = this.dataset.badgeId;
    const bodyId = section ? "section" + section[0].toUpperCase() + section.slice(1) : null;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    const card = document.createElement("div");
    card.className = "card";

    const header = document.createElement("div");
    header.className = "card-header";
    if (section) header.dataset.section = section;

    const h2 = document.createElement("h2");
    h2.textContent = title;
    if (badgeId) {
      h2.append(" ");
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.id = badgeId;
      badge.textContent = "0";
      h2.appendChild(badge);
    }
    header.appendChild(h2);

    const chevron = document.createElement("span");
    chevron.className = "chevron";
    chevron.innerHTML = "&#9660;";
    header.appendChild(chevron);

    const body = document.createElement("div");
    body.className = "card-body";
    if (bodyId) body.id = bodyId;
    body.appendChild(fragment);

    header.addEventListener("click", () => {
      const collapsed = body.classList.toggle("collapsed");
      header.classList.toggle("collapsed", collapsed);
    });

    card.appendChild(header);
    card.appendChild(body);
    this.appendChild(card);
  }
}

customElements.define("collapsible-card", CollapsibleCard);

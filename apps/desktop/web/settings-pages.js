/* Settings pages beyond the built-in ones register their content here (panel, system,
   webhooks, …), and the search box above every settings page opens any of them. */
(() => {
  const pages = new Map();
  const escape = (value) => AieyesUI.escapeHTML(value);
  // The setting to point at once the next settings render has finished.
  let pendingTitle = null,pendingSection = null;
  const part = (key, name, settings) => pages.get(key)?.[name]?.(settings) ?? '';
  const panel = () => document.querySelector('#settings-panel');
  function mark(element) {
    if (!element) return;
    element.scrollIntoView({ block: 'center', behavior: 'instant' });
    window.AieyesMotion?.highlight?.(element);
  }
  // A search hit lands on the row or card that carries the setting's label.
  function reveal() {
    const root = panel();if (!root) return;
    if (pendingSection) mark(root.querySelector(`[data-settings-section="${CSS.escape(pendingSection)}"]`));
    else if (pendingTitle) {
      const text = AieyesI18n.t(pendingTitle);
      const label = [...root.querySelectorAll('label,h2,h3,legend,button,summary')].find((el) => el.textContent.trim().startsWith(text));
      mark(label?.closest('.form-row,.setting-row,section,fieldset,.card') ?? label);
    }
    pendingTitle = pendingSection = null;
  }
  function results() {
    const input = document.querySelector('#settings-search'),list = document.querySelector('#settings-search-results');if (!input || !list) return [];
    const query = input.value.trim(),rows = AieyesSettingsIndex.search(query);
    list.hidden = !query;input.setAttribute('aria-expanded', String(Boolean(query)));
    document.querySelector('#content')?.toggleAttribute('data-settings-searching', Boolean(query));
    list.innerHTML = !query ? '' : rows.length ? rows.map((row, i) => `<button type="button" data-settings-result="${i}"><strong>${escape(AieyesI18n.t(row.title))}</strong><small>${escape(AieyesSettingsIndex.pagePath(row.category, row.page))}</small></button>`).join('') : `<p class="muted" role="status">${AieyesI18n.t("没有匹配项")}</p>`;
    list.querySelectorAll('[data-settings-result]').forEach((button) => button.onclick = () => open(rows[Number(button.dataset.settingsResult)]));
    return rows;
  }
  function open(row) {
    if (!row) return;
    state.settingsSearch = '';pendingTitle = row.category === 'general' ? row.title : null;
    void openSettings({ category: row.category, section: row.page ?? undefined }).then(reveal);
  }
  window.AieyesSettingsPages = {
    register(key, page) {pages.set(key, page);},
    has: (key) => !['panel', 'system', 'webhooks'].includes(key) || pages.has(key),
    fields: (key, settings) => part(key, 'fields', settings),
    body: (key, settings) => part(key, 'body', settings),
    searchHTML: () => `<div class="settings-search" role="search"><input type="search" id="settings-search" autocomplete="off" spellcheck="false" aria-controls="settings-search-results" aria-expanded="false" aria-label="${AieyesI18n.t("搜索设置")}" placeholder="${AieyesI18n.t("搜索设置")}" value="${escape(state.settingsSearch ?? '')}"><div id="settings-search-results" class="settings-search-results" hidden></div></div>`,
    // Opens a settings section by its data-settings-section key after the page has rendered.
    focusSection(section) {pendingSection = section;reveal();},
    bind(category, page) {
      const input = document.querySelector('#settings-search');
      if (input) {
        input.oninput = () => {state.settingsSearch = input.value;results();};
        input.onkeydown = (event) => {
          if (event.key === 'Enter') {event.preventDefault();open(results()[0]);}
          else if (event.key === 'Escape' && input.value) {event.preventDefault();event.stopPropagation();input.value = '';state.settingsSearch = '';results();}
          else if (event.key === 'ArrowDown') {const first = document.querySelector('[data-settings-result]');if (first) {event.preventDefault();first.focus();}}
        };
        results();
      }
      if (category === 'general') pages.get(page)?.bind?.();
    }
  };
})();

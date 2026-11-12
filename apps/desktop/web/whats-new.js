/* 本次更新: after an update, a banner on the overview (main window and panel) leads to the bundled
   release notes, shown under Settings › 更新与关于. The banner shows once per version until it is
   dismissed or the notes are opened; the notes stay available on that page for this version. */
(() => {
  // app.js declares `state` with const: a global binding, not a property of globalThis.
  const pageState = () => typeof state === 'undefined' ? undefined : state;
  const DISMISSED = 'aieyes.whatsNew.dismissed';
  let info = null;
  const escape = (value) => AieyesUI.escapeHTML(value);
  // The same structure as macOS ReleaseNotes: an English part, then a part after "## 中文";
  // the first heading is the headline, list lines are items, other lines are intro or outro.
  function parse(markdown) {
    const part = () => ({ headline: null, intro: [], items: [], outro: [] }),notes = { english: part(), chinese: part(), changelog: null };
    let target = notes.english;
    for (const raw of String(markdown ?? '').split(/\r?\n/)) {
      const line = raw.trim();if (!line) continue;
      if (line === `## 中文`) {target = notes.chinese;continue;}
      if (line.startsWith('#')) {const text = line.replace(/^#+/, '').trim();if (!target.headline && text) target.headline = text;}
      else if (line.startsWith('- ') || line.startsWith('* ')) target.items.push(line.slice(2));
      else if (/^\[.*\]\([^)\s]+\)$/.test(line)) notes.changelog = line.slice(line.lastIndexOf('](') + 2, -1);
      else (target.items.length ? target.outro : target.intro).push(line);
    }
    return notes.english.items.length || notes.chinese.items.length ? notes : null;
  }
  const part = (notes, chinese) => {const first = chinese ? notes.chinese : notes.english,second = chinese ? notes.english : notes.chinese;return first.items.length ? first : second;};
  // Inline markdown: `code` and **bold**; a link keeps its text only.
  const rich = (text) => escape(text).replace(/\[([^\]]+)\]\([^)]*\)/g, '$1').replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  const notes = () => info?.notes ? parse(info.notes) : null;
  const dismissed = () => {try {return localStorage.getItem(DISMISSED) === info?.version;} catch (_) {return false;}};
  function dismiss() {try {localStorage.setItem(DISMISSED, info?.version ?? '');} catch (_) {}sync();}
  function bannerHTML() {
    return `<div class="whats-new-banner" role="status">${uiIcon('sparkle')}<span>${AieyesI18n.t("已更新到 v{0}", escape(info.version))}</span><button type="button" data-whats-new-open>${AieyesI18n.t("查看本次更新")}</button><button type="button" class="icon-button" data-whats-new-dismiss aria-label="${AieyesI18n.t("关闭提示")}" title="${AieyesI18n.t("关闭提示")}">${uiIcon('x')}</button></div>`;
  }
  // Kept outside #content, so a live re-render of the overview does not move or replay it.
  function sync() {
    const content = document.querySelector('#content');if (!content) return;
    let slot = document.querySelector('#whats-new');
    const show = Boolean(info?.updated && notes() && !dismissed() && pageState()?.page === 'agent');
    if (!show) {if (slot) slot.hidden = true;return;}
    if (!slot) {slot = document.createElement('div');slot.id = 'whats-new';content.before(slot);}
    if (!slot.firstElementChild) slot.innerHTML = bannerHTML();
    slot.hidden = false;
    slot.querySelector('[data-whats-new-open]').onclick = () => {dismiss();void openSettings({ category: 'general', section: 'updates', focus: 'release-notes' });};
    slot.querySelector('[data-whats-new-dismiss]').onclick = dismiss;
  }
  function sectionHTML() {
    const parsed = notes();if (!parsed) return '';
    const p = part(parsed, (state.settings?.appearance?.language ?? 'zh-CN') !== 'en');
    return `<section class="card release-notes" data-settings-section="release-notes"><h2>${AieyesI18n.t("本次更新")}</h2><p class="muted tiny">Aieyes v${escape(info.version)}${p.headline ? ' · ' + escape(p.headline) : ''}</p>${p.intro.map((line) => `<p class="muted">${rich(line)}</p>`).join('')}<ul class="release-notes-items">${p.items.map((item) => `<li>${rich(item)}</li>`).join('')}</ul>${p.outro.map((line) => `<p class="muted tiny">${rich(line)}</p>`).join('')}${info.changelog ? `<div class="actions"><button type="button" id="release-notes-changelog">${AieyesI18n.t("完整变更记录")}</button></div>` : ''}</section>`;
  }
  async function load() {
    if (!window.__TAURI__) return;
    try {info = await window.__TAURI__.core.invoke('whats_new_info');} catch (_) {info = null;}
    sync();if (pageState()?.page === 'settings' && state.generalTab === 'updates') renderSettings(false);
  }
  window.addEventListener('storage', (event) => {if (event.key === DISMISSED) sync();});
  window.AieyesSettingsPages?.register('updates', {
    body: () => info ? sectionHTML() : '',
    bind() {
      const button = document.querySelector('#release-notes-changelog');
      if (button) button.onclick = () => window.__TAURI__.core.invoke('whats_new_open_changelog').catch((error) => notify(String(error), 'error'));
    }
  });
  window.AieyesWhatsNew = { parse, part, sync, get info() {return info;} };
  // After every script has run (app.js defines the page state).
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => void load(), { once: true });else void load();
})();

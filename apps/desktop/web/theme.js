/* Saved application appearance, shared by every web surface. */
(() => {
  // Start loading the bundled faces before IPC/layout on Windows and Linux.
  if (typeof navigator !== 'undefined') {
    const platform = navigator.userAgentData?.platform || navigator.platform || '';
    if (/Win/i.test(platform)) document.documentElement.dataset.platform = 'windows';
    else if (/Linux/i.test(platform)) document.documentElement.dataset.platform = 'linux';
  }
  const media = matchMedia('(prefers-color-scheme: dark)');
  let nativeSystemDark = null;
  let current = { theme: 'system', accent: 'system' },nativeKey = '',storedKey = '';
  try {current = JSON.parse(localStorage.getItem('aieyes.appearance') || 'null') || current;} catch (_) {}
  let busy = false;
  let fonts = null;
  function loadFonts() {
    if (!['windows', 'linux'].includes(document.documentElement.dataset.platform) || !document.fonts) return Promise.resolve();
    if (!fonts) fonts = Promise.allSettled([400, 500, 700].map((weight) => document.fonts.load(`${weight} 15px "Aieyes HarmonyOS Sans SC"`, AieyesI18n.t("今日概览 Token 0123456789")))).then(() => {
      window.dispatchEvent(new CustomEvent('aieyes:fonts'));
    });
    return fonts;
  }
  function updateButtons() {
    // Three choices, so following the system is never lost by a quick switch (§3.7).
    const dark = document.documentElement.dataset.theme === 'dark',names = { system: AieyesI18n.t("跟随系统"), light: AieyesI18n.t("浅色"), dark: AieyesI18n.t("深色") };
    const label = AieyesI18n.t("主题：{0}", names[current.theme] ?? names.system),icon = current.theme === 'system' ? 'themeSystem' : dark ? 'moon' : 'sun';
    document.querySelectorAll('[data-theme-menu]').forEach((menu) => {
      const trigger = menu.querySelector('summary');
      if (trigger.dataset.icon !== icon) {trigger.dataset.icon = icon;trigger.innerHTML = globalThis.uiIcon?.(icon) ?? '';}
      if (trigger.title !== label) {trigger.title = label;trigger.setAttribute('aria-label', label);}menu.toggleAttribute('data-busy', busy);
      menu.querySelectorAll('[data-theme-choice]').forEach((item) => {const checked = String(item.dataset.themeChoice === current.theme);if (item.getAttribute('aria-checked') !== checked) item.setAttribute('aria-checked', checked);item.disabled = busy;});
    });
  }
  function apply(value = current, native = true) {
    const previousPreference = JSON.stringify(current);
    current = { language: window.AieyesI18n.normalize(value?.language), theme: ['system', 'light', 'dark'].includes(value?.theme) ? value.theme : 'system', accent: ['system', 'indigo', 'blue', 'teal', 'purple'].includes(value?.accent) ? value.accent : 'system', chartPalette: window.AieyesChartPalettes.some((p) => p.id === value?.chartPalette) ? value.chartPalette : 'classic', chartContrast: value?.chartContrast === 'low' ? 'low' : 'high' };
    const root = document.documentElement,set = (name, value) => {if (root.dataset[name] !== value) root.dataset[name] = value;};
    const previous = root.dataset.theme + root.dataset.accent;
    set('theme', current.theme === 'system' ? (nativeSystemDark ?? media.matches) ? 'dark' : 'light' : current.theme);
    set('accent', current.accent);
    set('chartPalette', current.chartPalette);
    set('chartContrast', current.chartContrast);
    // Adopt saved settings before the language event renders a sibling window.
    if (previous !== root.dataset.theme + root.dataset.accent || previousPreference !== JSON.stringify(current)) window.dispatchEvent(new CustomEvent('aieyes:appearance', { detail: { ...current } }));
    window.AieyesI18n.apply(current.language);
    updateButtons();
    const key = JSON.stringify(current);
    if (storedKey !== key) {storedKey = key;try {localStorage.setItem('aieyes.appearance', key);} catch (_) {}}
    if (native && window.__TAURI__?.core && nativeKey !== key) {nativeKey = key;window.__TAURI__.core.invoke('desktop_appearance', { theme: current.theme, accent: current.accent, language: current.language }).catch(() => {nativeKey = '';});}
  }
  window.__TAURI__?.event?.listen('desktop:system-theme', ({payload}) => {
    nativeSystemDark = payload.dark === true;apply(current, false);
  }).catch(() => {});
  media.addEventListener('change' , () => apply(current));
  window.addEventListener('storage', (e) => {if (e.key === 'aieyes.appearance') {try {apply(JSON.parse(e.newValue));} catch (_) {}}});
  document.addEventListener('DOMContentLoaded', () => {updateButtons();loadFonts();});
  window.AieyesTheme = { apply, loadFonts, setBusy(value) {busy = value;updateButtons();}, updateButtons };
  // The native shell reads the saved appearance itself at startup. This cached copy can be stale (localStorage
  // commits late, so a crash loses recent changes) and would revert the tray and pet until the settings load.
  apply(current, false);
})();

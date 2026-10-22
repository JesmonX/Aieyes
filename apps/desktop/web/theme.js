/* Saved application appearance, shared by every web surface. */
(() => {
  // Start loading the bundled faces before IPC/layout on an actual Windows host.
  if (typeof navigator !== 'undefined' && /Win/i.test(navigator.userAgentData?.platform || navigator.platform || '')) document.documentElement.dataset.platform = 'windows';
  const media = matchMedia('(prefers-color-scheme: dark)');
  let current = { theme: 'system', accent: 'indigo' },nativeKey = '';
  try {current = JSON.parse(localStorage.getItem('aieyes.appearance') || 'null') || current;} catch (_) {}
  let busy = false;
  let fonts = null;
  function loadFonts() {
    if (document.documentElement.dataset.platform !== 'windows' || !document.fonts) return Promise.resolve();
    if (!fonts) fonts = Promise.allSettled([400, 500, 700].map((weight) => document.fonts.load(`${weight} 15px "Aieyes HarmonyOS Sans SC"`, AieyesI18n.t("今日概览 Token 0123456789")))).then(() => {
      window.dispatchEvent(new CustomEvent('aieyes:fonts'));
    });
    return fonts;
  }
  function updateButtons() {
    const dark = document.documentElement.dataset.theme === 'dark',label = dark ? AieyesI18n.t("切换到浅色模式") : AieyesI18n.t("切换到深色模式");
    document.querySelectorAll('[data-theme-toggle]').forEach((button) => {button.innerHTML = globalThis.uiIcon?.(dark ? 'sun' : 'moon') ?? '';button.title = label;button.setAttribute('aria-label', label);button.disabled = busy;});
  }
  function apply(value = current) {
    const previousPreference = JSON.stringify(current);
    current = { language: window.AieyesI18n.normalize(value?.language), theme: ['system', 'light', 'dark'].includes(value?.theme) ? value.theme : 'system', accent: ['indigo', 'blue', 'teal', 'purple'].includes(value?.accent) ? value.accent : 'indigo', chartPalette: window.AieyesChartPalettes.some((p) => p.id === value?.chartPalette) ? value.chartPalette : 'classic', chartContrast: value?.chartContrast === 'low' ? 'low' : 'high' };
    const root = document.documentElement;
    const previous = root.dataset.theme + root.dataset.accent;
    root.dataset.theme = current.theme === 'system' ? media.matches ? 'dark' : 'light' : current.theme;
    root.dataset.accent = current.accent;
    root.dataset.chartPalette = current.chartPalette;
    root.dataset.chartContrast = current.chartContrast;
    // Adopt saved settings before the language event renders a sibling window.
    if (previous !== root.dataset.theme + root.dataset.accent || previousPreference !== JSON.stringify(current)) window.dispatchEvent(new CustomEvent('aieyes:appearance', { detail: { ...current } }));
    window.AieyesI18n.apply(current.language);
    updateButtons();
    try {localStorage.setItem('aieyes.appearance', JSON.stringify(current));} catch (_) {}
    const key = JSON.stringify(current);
    if (window.__TAURI__?.core && nativeKey !== key) {nativeKey = key;window.__TAURI__.core.invoke('desktop_appearance', { theme: current.theme, accent: current.accent, language: current.language }).catch(() => {nativeKey = '';});}
  }
  media.addEventListener('change', () => apply(current));
  window.addEventListener('storage', (e) => {if (e.key === 'aieyes.appearance') {try {apply(JSON.parse(e.newValue));} catch (_) {}}});
  document.addEventListener('DOMContentLoaded', () => {updateButtons();loadFonts();});
  window.AieyesTheme = { apply, loadFonts, setBusy(value) {busy = value;updateButtons();}, updateButtons };
  apply();
})();

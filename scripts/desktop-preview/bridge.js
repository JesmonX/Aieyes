/* Injected into development responses before application scripts. */
(() => {
  const host = window.parent.PreviewHost;
  if (!host || window.parent === window) {location.replace('/');return;}
  const binding = host.attach(Number(new URL(location.href).searchParams.get('generation')));
  if (!binding) return;
  const rawStorage = window.localStorage;
  const prefix = `aieyes.preview.v1.ui.${binding.key}.`;
  const keys = () => Object.keys(rawStorage).filter(k => k.startsWith(prefix)).map(k => k.slice(prefix.length));
  const storage = new Proxy({}, {
    get(_, name) {
      if (name === 'getItem') return k => rawStorage.getItem(prefix + k);
      if (name === 'setItem') return (k, v) => rawStorage.setItem(prefix + k, String(v));
      if (name === 'removeItem') return k => rawStorage.removeItem(prefix + k);
      if (name === 'clear') return () => keys().forEach(k => rawStorage.removeItem(prefix + k));
      if (name === 'key') return i => keys()[i] ?? null;
      if (name === 'length') return keys().length;
      if (typeof name === 'string') return rawStorage.getItem(prefix + name);
    },
    set(_, name, value) {rawStorage.setItem(prefix + name, String(value));return true;},
    ownKeys: keys,
    getOwnPropertyDescriptor(_, name) {if (keys().includes(name)) return { enumerable: true, configurable: true };},
    deleteProperty(_, name) {rawStorage.removeItem(prefix + name);return true;},
  });
  Object.defineProperty(window, 'localStorage', { value: storage });
  storage.setItem('aieyes.appearance', JSON.stringify(binding.appearance));storage.setItem('aieyes.language', binding.appearance.language);
  document.documentElement.dataset.platform = binding.platform;
  const cleanup = [];
  const listen = async (name, fn) => {const off = host.listen(binding.id, name, fn);cleanup.push(off);return off;};
  const native = {
    isMaximized: async () => host.maximized,
    toggleMaximize: async () => host.windowAction(binding.id, 'maximize'),
    minimize: async () => host.windowAction(binding.id, 'minimize'),
    close: async () => host.windowAction(binding.id, 'close'),
    startDragging: async () => host.note('窗口拖动在浏览器中仅作模拟，请使用尺寸控件调整画布。'),
    onResized: async fn => {window.addEventListener('resize', fn);const off = () => window.removeEventListener('resize', fn);cleanup.push(off);return off;},
  };
  window.__TAURI__ = { core: { invoke: async (command, args) => structuredClone(await host.invoke(binding.id, command, structuredClone(args))) }, event: { listen }, window: { getCurrentWindow: () => native } };
  window.addEventListener('error', e => host.report(binding.id, e.error?.stack || e.message));
  window.addEventListener('unhandledrejection', e => host.report(binding.id, String(e.reason?.stack || e.reason)));
  window.addEventListener('pagehide', () => cleanup.splice(0).forEach(off => off()), { once: true });
  window.addEventListener('DOMContentLoaded', async () => {
    try {await window.AieyesApp?.ready;await window.AieyesTheme?.loadFonts();host.ready(binding.id);} catch (error) {host.report(binding.id, String(error));}
  }, { once: true });
})();

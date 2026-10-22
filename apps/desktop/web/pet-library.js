/* Library metadata is cheap; sprite sheets are requested only for visible previews. */
window.AieyesPetLibrary = (() => {
  const e = (value) => AieyesUI.escapeHTML(value ?? ''),reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const library = { pets: [], selectedLibraryId: null, warnings: [], loaded: false, input: '', status: '', error: '', busy: false, lastInput: '' };
  const assets = new Map(),previews = new Map();
  let root = null,observer = null,removalObserver = null,timer = null,loading = null,retryAction = null,retryLabel = AieyesI18n.t("重试"),revision = 0;window.addEventListener("aieyes:language", () => {retryLabel = AieyesI18n.t("重试");});
  const label = (value) => typeof value === 'string' ? value : value?.name ?? value?.url ?? '';
  const imported = (pet) => {const date = new Date(pet.importedAt);return Number.isFinite(date.getTime()) ? date.toLocaleString(undefined, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : '';};
  function html() {return `<section class="pet-library" id="pet-library"><h3>${AieyesI18n.t("我的桌宠")}</h3><p id="current-pet" class="muted tiny" role="status">${AieyesI18n.t("正在读取桌宠…")}</p><form id="pet-import-form"><label for="pet-import-input">${AieyesI18n.t("链接、npx 命令或 npm 包名")}</label><input id="pet-import-input" type="text" value="${e(library.input)}" placeholder="https://… / npx … / @scope/package" autocomplete="off" spellcheck="false"><div class="actions"><button id="import-pet" type="submit">${AieyesI18n.t("导入并使用")}</button><button id="select-pet-file" type="button">${AieyesI18n.t("选择本地文件…")}</button></div></form><p class="muted tiny">${AieyesI18n.t("支持 ZIP、pet.json、桌宠链接、npx 命令和 npm 包名。导入后立即使用，已有桌宠保留。")}</p><p id="pet-import-status" role="status"></p><button id="pet-import-retry" type="button" hidden>${AieyesI18n.t("重试导入")}</button><div id="pet-library-warnings"></div><div id="pet-library-list" class="pet-library-grid"></div></section>`;}
  function status() {
    if (!root?.isConnected) return;
    const line = root.querySelector('#pet-import-status');if (library.error) AieyesUI.showError(line, library.error, AieyesI18n.t("桌宠操作失败"));else line.textContent = AieyesI18n.message(library.status);line.className = library.error ? 'error' : 'muted tiny';
    const retry = root.querySelector('#pet-import-retry');retry.hidden = !library.error || !retryAction;retry.disabled = library.busy;retry.textContent = retryLabel;
    for (const id of ['import-pet', 'select-pet-file']) root.querySelector('#' + id).disabled = library.busy;
    root.querySelector('#import-pet').textContent = library.busy ? AieyesI18n.t("处理中…") : AieyesI18n.t("导入并使用");
    root.querySelectorAll('[data-pet-select]').forEach((button) => {const selected = button.dataset.petSelect === library.selectedLibraryId;button.disabled = library.busy || selected;button.textContent = selected ? AieyesI18n.t("使用中") : AieyesI18n.t("使用");button.setAttribute('aria-pressed', String(selected));});
    const current = library.pets.find((p) => p.libraryId === library.selectedLibraryId);
    root.querySelector('#current-pet').textContent = current ? AieyesI18n.t("当前桌宠：") + current.displayName : library.loaded ? AieyesI18n.t("导入桌宠后在这里切换") : AieyesI18n.t("正在读取桌宠…");
  }
  function renderList() {
    if (!root?.isConnected) return;
    stopPreviews();
    const list = root.querySelector('#pet-library-list');
    list.innerHTML = library.pets.map((pet) => {
      const duplicate = library.pets.filter((other) => other.displayName === pet.displayName).length > 1;
      const meta = [label(pet.creator), label(pet.source)].filter(Boolean).join(' · ');
      return `<article class="pet-library-card" data-pet-id="${e(pet.libraryId)}"><canvas width="88" height="96" aria-label="${AieyesI18n.t("{0}空闲动画预览", e(pet.displayName))}"></canvas><div class="pet-library-info"><strong title="${e(pet.displayName)}">${e(pet.displayName)}</strong>${meta ? `<small title="${e(meta)}">${e(meta)}</small>` : ''}${duplicate || pet.displayName.startsWith('历史桌宠') ? `<small>${e(imported(pet) || pet.libraryId.slice(0, 12))}</small>` : ''}<span class="pet-preview-error error" hidden></span></div><button type="button" data-pet-select="${e(pet.libraryId)}" aria-label="${AieyesI18n.t("使用 {0}", e(pet.displayName))}">${AieyesI18n.t("使用")}</button></article>`;
    }).join('') || (library.loaded ? AieyesI18n.t("<p class=\"muted\">还没有桌宠。</p>") : AieyesI18n.t("<p class=\"muted\">正在读取列表…</p>"));
    root.querySelector('#pet-library-warnings').innerHTML = (library.warnings ?? []).map((w) => `<div class="error">${e(w.path)}：${e(AieyesI18n.message(w.error))}</div>`).join('');
    list.querySelectorAll('[data-pet-select]').forEach((button) => button.onclick = () => select(button.dataset.petSelect));
    observePreviews();status();
  }
  async function refresh(afterMutation = false) {
    if (loading) {if (!afterMutation) return loading;await loading;}
    const requested = revision;
    loading = (async () => {
      try {
        const result = await api('pets.list');if (requested !== revision) return;
        if (retryLabel === '重试读取列表') library.error = '';
        const changed = !library.loaded || JSON.stringify([library.pets, library.warnings]) !== JSON.stringify([result?.pets ?? [], result?.warnings ?? []]);
        if (library.loaded && !library.busy && library.selectedLibraryId !== result?.selectedLibraryId) library.status = '';
        library.pets = result?.pets ?? [];library.selectedLibraryId = result?.selectedLibraryId ?? null;library.warnings = result?.warnings ?? [];library.loaded = true;
        if (changed) renderList();else status();
      } catch (error) {if (requested === revision) {library.error = AieyesI18n.t("桌宠列表读取失败：") + String(error.message ?? error);retryAction = refresh;retryLabel = AieyesI18n.t("重试读取列表");status();}} finally
      {loading = null;}
    })();return loading;
  }
  async function importPet(input) {
    if (library.busy) return;
    input = input.trim();if (!input) {retryAction = null;library.error = AieyesI18n.t("请填写链接、npx 命令或 npm 包名，或选择本地文件。");status();root?.querySelector('#pet-import-input')?.focus();return;}
    library.busy = true;retryAction = null;library.lastInput = input;library.error = '';library.status = AieyesI18n.t("正在识别并导入桌宠…");status();
    try {
      const result = await api('pets.import', { input });if (result.error) throw new Error(result.error);
      if (!result.pet) throw new Error(AieyesI18n.t("导入未返回有效桌宠，请重试。"));
      revision++;library.selectedLibraryId = result.pet.libraryId;
      library.status = AieyesI18n.t("已导入并使用 ") + result.pet.displayName;
      if (library.input === input) {library.input = '';if (root?.isConnected) root.querySelector('#pet-import-input').value = '';}
      await refresh(true);
    } catch (error) {library.error = String(error.message ?? error);library.status = '';retryAction = () => importPet(input);retryLabel = AieyesI18n.t("重试导入");} finally
    {library.busy = false;status();}
  }
  async function select(libraryId) {
    if (library.busy) return;library.busy = true;retryAction = null;library.error = '';library.status = AieyesI18n.t("正在切换桌宠…");status();
    try {
      const result = await api('pets.select', { libraryId });if (result.error) throw new Error(result.error);
      if (!result.pet) throw new Error(AieyesI18n.t("此桌宠不可用，请重新导入。"));
      revision++;library.selectedLibraryId = result.pet.libraryId;library.status = AieyesI18n.t("正在使用 ") + result.pet.displayName;
      await refresh(true);
    } catch (error) {library.error = String(error.message ?? error);library.status = '';retryAction = () => select(libraryId);retryLabel = AieyesI18n.t("重试切换");} finally
    {library.busy = false;status();}
  }
  function asset(pet) {
    if (!assets.has(pet.libraryId)) assets.set(pet.libraryId, (async () => {
      const result = await api('pets.asset', { libraryId: pet.libraryId });if (result.error) throw new Error(result.error);
      if (!result.pet || !result.imageDataUrl) throw new Error(AieyesI18n.t("预览图片不可用"));
      const sheet = new Image();sheet.src = result.imageDataUrl;await sheet.decode();
      const { cellWidth, cellHeight } = result.pet;
      return Array.from({ length: AieyesPetAnimation.idle().frames }, (_, index) => {
        const canvas = document.createElement('canvas');canvas.width = cellWidth;canvas.height = cellHeight;
        canvas.getContext('2d').drawImage(sheet, index * cellWidth, 0, cellWidth, cellHeight, 0, 0, cellWidth, cellHeight);return canvas;
      });
    })().catch((error) => {assets.delete(pet.libraryId);throw error;}));
    return assets.get(pet.libraryId);
  }
  function paintPreview(preview, now) {
    if (!preview.frames) return;
    const index = AieyesPetAnimation.frame(AieyesPetAnimation.idle(), now - preview.started, reduced.matches);
    const density = window.devicePixelRatio || 1,width = Math.round(88 * density),height = Math.round(96 * density),signature = [index, width, height].join(':');
    if (preview.painted === signature) return;preview.painted = signature;
    const canvas = preview.canvas;if (canvas.width !== width || canvas.height !== height) {canvas.width = width;canvas.height = height;}
    const context = canvas.getContext('2d');context.clearRect(0, 0, width, height);context.imageSmoothingEnabled = true;context.imageSmoothingQuality = 'high';
    const frame = preview.frames[index];if (frame) context.drawImage(frame, 0, 0, width, height);canvas.dataset.frame = String(index);
  }
  function tick(now = performance.now()) {
    timer = null;if (!root?.isConnected) {dispose();return;}
    if (document.hidden) return;
    let active = false;
    for (const preview of previews.values()) if (preview.visible && preview.frames) {paintPreview(preview, now);active = true;}
    if (active && !reduced.matches) timer = requestAnimationFrame(tick);
  }
  function schedule() {if (timer !== null) cancelAnimationFrame(timer);timer = null;tick();}
  function observePreviews() {
    observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const preview = previews.get(entry.target);if (!preview) continue;
        const was = preview.visible;preview.visible = entry.isIntersecting;
        if (preview.visible && !was) preview.started = performance.now();
        if (preview.visible && !preview.frames && !preview.loading) {
          preview.loading = true;
          void asset(preview.pet).then((frames) => {preview.frames = frames;schedule();}).catch((error) => {
            if (!preview.canvas.isConnected) return;const line = preview.canvas.parentElement.querySelector('.pet-preview-error');line.textContent = AieyesI18n.message(String(error.message ?? error));line.hidden = false;
          }).finally(() => preview.loading = false);
        }
      }
      schedule();
    }, { threshold: 0.01 });
    for (const canvas of root.querySelectorAll('canvas')) {
      const pet = library.pets.find((p) => p.libraryId === canvas.parentElement.dataset.petId);if (!pet) continue;
      previews.set(canvas, { canvas, pet, visible: false, started: performance.now(), frames: null, loading: false, painted: null });observer.observe(canvas);
    }
  }
  function stopPreviews() {if (timer !== null) cancelAnimationFrame(timer);timer = null;observer?.disconnect();observer = null;previews.clear();}
  function dispose() {stopPreviews();removalObserver?.disconnect();removalObserver = null;root = null;}
  function bind() {
    dispose();root = document.querySelector('#pet-library');if (!root) return;
    const input = root.querySelector('#pet-import-input');input.oninput = () => library.input = input.value;
    root.querySelector('#pet-import-form').onsubmit = (event) => {event.preventDefault();void importPet(input.value);};
    root.querySelector('#pet-import-retry').onclick = () => retryAction?.();
    root.querySelector('#select-pet-file').onclick = async () => {
      try {const path = await window.__TAURI__.core.invoke('select_local_path', { directory: false, initial: '~' });if (path) await importPet(path);}
      catch (error) {library.error = String(error.message ?? error);status();}
    };
    removalObserver = new MutationObserver(() => {if (root && !root.isConnected) dispose();});removalObserver.observe(document.querySelector('#content'), { childList: true });
    renderList();void refresh();
  }
  reduced.addEventListener('change', schedule);document.addEventListener('visibilitychange', schedule);window.addEventListener('pagehide', dispose);
  return { html, bind, dispose };
})();

'use strict';

let monacoPromise;
function loadMonaco() {
  if (monacoPromise) return monacoPromise;
  monacoPromise = new Promise((resolve, reject) => {
    const base = 'harbor://' + location.hostname + '/monaco/vs';
    const loader = document.createElement('script');
    const timer = setTimeout(() => reject(new Error('Monaco Editor could not be loaded.')), 10000);
    const fail = () => {
      clearTimeout(timer);
      reject(new Error('Monaco Editor could not be loaded.'));
    };
    loader.src = base + '/loader.js';
    loader.onerror = fail;
    loader.onload = () => {
      window.MonacoEnvironment = { getWorkerUrl: () => base + '/base/worker/workerMain.js' };
      window.require.config({ paths: { vs: base } });
      window.require(['vs/editor/editor.main'], () => {
        clearTimeout(timer);
        resolve(window.monaco);
      }, fail);
    };
    document.head.append(loader);
  });
  return monacoPromise;
}

function mountCodeEditor(surface, fallback, status, signal, readOnly = false) {
  let editor;
  loadMonaco().then(monaco => {
    if (signal.aborted) return;
    const hadFocus = document.activeElement === fallback;
    const model = monaco.editor.createModel(fallback.value, 'javascript');
    surface.hidden = false;
    try {
      editor = monaco.editor.create(surface, {
        model, theme: 'vs-dark', readOnly,
        ariaLabel: readOnly ? 'User script source, read only' : 'JavaScript source',
        automaticLayout: true, minimap: { enabled: false }, tabSize: 2,
        scrollBeyondLastLine: false
      });
    } catch (error) {
      model.dispose();
      surface.hidden = true;
      throw error;
    }
    signal.addEventListener('abort', () => {
      editor.dispose();
      model.dispose();
      surface.hidden = true;
      fallback.hidden = false;
    }, { once: true });
    fallback.hidden = true;
    if (hadFocus) editor.focus();
  }).catch(error => {
    if (!signal.aborted) setStatus(status, error.message + ' The text editor is available.', true);
  });
  return () => editor ? editor.getValue() : fallback.value;
}

function renderScriptsDashboard(view, signal) {
  const template = '// ==UserScript==\n// @name New script\n// @match https://example.com/*\n// @grant none\n// ==/UserScript==\n\n';
  view.innerHTML = `<header class="page-header"><h1>User Scripts</h1></header>
    <div class="scripts-toolbar">
      <form class="scripts-url-form" id="remote-script">
        <div class="field-action">
          <input class="input" id="remote-script-url" name="url" type="url" required
            aria-label="Install from URL" placeholder="Install from URL (https://example.com/script.user.js)"
            aria-describedby="remote-script-status">
          <button class="btn icon-control" type="submit" aria-label="Install from URL" title="Install from URL">${icon('plus')}</button>
        </div>
      </form>
      <button class="btn primary" type="button" id="create-script">Create script</button>
      <a class="btn" href="https://greasyfork.org/">Browse Greasy Fork</a>
    </div>
    <p class="status" id="remote-script-status" role="status"></p>
    <section class="panel" id="edit-script-panel" hidden>
      <h2 id="script-editor-title">New script</h2>
      <div class="editor" id="monaco-editor" hidden></div>
      <textarea class="textarea source-review" id="script-text" aria-label="JavaScript source" spellcheck="false"></textarea>
      <p class="status" id="editor-status" role="status"></p>
      <div class="row wrap compact"><button class="btn primary" id="save-script">Save script</button>
        <button class="btn" id="cancel-script">Cancel</button></div>
    </section>
    <section class="settings-section" aria-labelledby="installed-scripts-heading">
      <h2 id="installed-scripts-heading">Installed scripts</h2>
      <p class="status" id="script-list-status" role="status">Loading scripts…</p>
      <div class="script-list" id="installed-scripts"></div>
    </section>`;
  const status = view.querySelector('#script-list-status');
  const editStatus = view.querySelector('#editor-status');
  const panel = view.querySelector('#edit-script-panel');
  const fallback = view.querySelector('#script-text');
  const editorSurface = view.querySelector('#monaco-editor');
  const save = view.querySelector('#save-script');
  const cancel = view.querySelector('#cancel-script');
  const create = view.querySelector('#create-script');
  const list = view.querySelector('#installed-scripts');
  let editingId = null;
  let editorLifetime;
  let readSource;
  let returnFocus;

  function openEditor(source, id, title, trigger) {
    if (save.disabled) return;
    editorLifetime?.abort();
    editorLifetime = new AbortController();
    editingId = id;
    returnFocus = trigger;
    view.querySelector('#script-editor-title').textContent = title;
    panel.hidden = false;
    setStatus(editStatus, '');
    fallback.value = source;
    fallback.focus();
    readSource = mountCodeEditor(editorSurface, fallback, editStatus, AbortSignal.any([signal, editorLifetime.signal]));
  }

  function closeEditor() {
    editorLifetime?.abort();
    editingId = null;
    fallback.value = '';
    panel.hidden = true;
    (returnFocus?.isConnected ? returnFocus : create).focus();
  }

  async function load() {
    const scripts = await callWorkspace('state');
    if (signal.aborted) return;
    list.innerHTML = scripts.map(script => `<div class="listitem">
      <div class="break"><h3>${escapeText(script.name)}</h3><small><span data-script-state>${script.enabled ? 'Enabled' : 'Disabled'}</span>
        ${script.version ? ' · ' + escapeText(script.version) : ''}</small></div>
      <div class="script-details break"><span class="script-detail-label">Permissions</span>
        ${escapeText(script.grants.filter(grant => grant !== 'none').join(', ') || 'None declared')}</div>
      <div class="script-details break"><span class="script-detail-label">Matching websites</span>${escapeText(script.matches.join(', '))}
        ${script.excludes.length ? '<small>Except ' + escapeText(script.excludes.join(', ')) + '</small>' : ''}</div>
      <div class="actions">${iconToggle('enabled', 'Enable ' + script.name, script.enabled, script.id)}
        ${actionButton('edit', 'Edit script', script.id)}${actionButton('delete', 'Delete script', script.id)}</div>
    </div>`).join('') || '<p class="empty-state">No user scripts installed.</p>';
    list.querySelectorAll('.listitem').forEach((row, index) => {
      const script = scripts[index];
      const edit = row.querySelector('[data-action=edit]');
      const remove = row.querySelector('[data-action=delete]');
      const toggle = row.querySelector('[data-action=enabled]');
      toggle.onclick = runAction(toggle, status, async () => {
        const enabled = toggle.getAttribute('aria-pressed') !== 'true';
        await callWorkspace('set_script_enabled', { id: script.id, enabled });
        toggle.setAttribute('aria-pressed', String(enabled));
        row.querySelector('[data-script-state]').textContent = enabled ? 'Enabled' : 'Disabled';
        setStatus(status, enabled ? 'Script enabled.' : 'Script disabled.');
      });
      edit.onclick = runAction(edit, status, async () => {
        const details = await callWorkspace('script_details', { id: script.id });
        if (signal.aborted) return;
        openEditor(details.source, script.id, 'Edit ' + script.name, edit);
        setStatus(status, '');
      });
      remove.onclick = runAction(remove, status, async () => {
        if (await confirmDashboard('Delete script?', script.name, 'Delete script')) {
          await callWorkspace('delete_record', { id: script.id });
          if (editingId === script.id) closeEditor();
          await load();
          restoreRowFocus(list, index, '[data-action=edit]', create);
          setStatus(status, 'Script deleted.');
        } else setStatus(status, '');
      });
    });
    setStatus(status, '');
  }

  const install = view.querySelector('#remote-script');
  install.onsubmit = runAction(install.querySelector('button'), view.querySelector('#remote-script-status'), () => {
    const source = new URL(install.elements.url.value.trim());
    if (source.protocol !== 'https:' || source.username || source.password) {
      throw new Error('Enter a raw HTTPS script URL.');
    }
    const page = new URL('harbor://install-script/');
    page.searchParams.set('url', source.href);
    location.href = page.href;
  });
  create.onclick = () => openEditor(template, null, 'New script', create);
  cancel.onclick = closeEditor;
  save.onclick = runAction(save, editStatus, async () => {
    const id = editingId;
    cancel.disabled = create.disabled = true;
    try {
      await callWorkspace('save_script', { id, source: readSource() });
      closeEditor();
      await load();
      const edit = [...list.querySelectorAll('[data-action=edit]')].find(button => button.dataset.id === id);
      create.disabled = false;
      (edit || create).focus();
      setStatus(status, 'Script saved.');
    } finally {
      cancel.disabled = create.disabled = false;
    }
  });
  load().catch(error => setStatus(status, error.message, true));
}

function renderScriptReview(view, signal) {
  view.innerHTML = `<header class="page-header"><h1>Review user script</h1><button class="btn" id="script-cancel">Cancel</button></header>
    <p role="status" id="script-status">Fetching the script…</p><div id="script-review"></div>`;
  const status = view.querySelector('#script-status');
  const cancel = view.querySelector('#script-cancel');
  cancel.onclick = () => setRoute('scripts');
  window.__harborScriptReview = ({ data: review, error }) => {
    if (signal.aborted) return;
    const details = view.querySelector('#script-review');
    if (error) {
      setStatus(status, error, true);
      const retry = document.createElement('button');
      retry.className = 'btn';
      retry.textContent = 'Try again';
      retry.onclick = () => location.reload();
      details.replaceChildren(retry);
      return;
    }
    details.innerHTML = `
      <div class="panel">
        <h2>${escapeText(review.script.name)}</h2>
        <dl class="review-details">
          <div><dt>Source</dt><dd>${escapeText(review.source_url)}${review.final_url !== review.source_url ? '<br>Downloaded from ' + escapeText(review.final_url) : ''}</dd></div>
          <div><dt>Version</dt><dd>${escapeText(review.script.version || 'Unspecified')}${review.previous_version ? '<br>Installed version ' + escapeText(review.previous_version) : ''}</dd></div>
          <div><dt>Runs on</dt><dd>${escapeText(review.script.matches.join(', '))}
            ${review.script.excludes.length ? '<br>Except ' + escapeText(review.script.excludes.join(', ')) : ''}</dd></div>
          <div><dt>Declared permissions</dt><dd>${escapeText(review.script.grants.filter(grant => grant !== 'none').join(', ') || 'None declared')}</dd></div>
        </dl>
        <label for="script-source">Source code</label>
        <div class="editor" id="review-editor" hidden></div>
        <textarea class="textarea source-review" id="script-source" readonly spellcheck="false"></textarea>
        <p class="status" id="review-editor-status" role="status"></p>
        <button class="btn primary" id="script-approve">${escapeText(review.action)} script</button>
      </div>`;
    const fallback = details.querySelector('#script-source');
    fallback.value = review.source;
    mountCodeEditor(details.querySelector('#review-editor'), fallback, details.querySelector('#review-editor-status'), signal, true);
    setStatus(status, 'User scripts can read and change matching pages. Review the source before approving.');
    const approve = details.querySelector('#script-approve');
    approve.onclick = runAction(approve, status, async () => {
      cancel.disabled = true;
      try {
        await callWorkspace('install_script', { token: review.token });
        setRoute('scripts');
      } finally {
        cancel.disabled = false;
      }
    });
  };
}

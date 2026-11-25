'use strict';

function describeExtensionPermission(perm) {
  const map = {
    storage: 'Store extension settings and data locally',
    tabs: 'Read open tab titles and URLs',
    activeTab: 'Access the current tab when you invoke the extension',
    declarativeNetRequest: 'Block or modify network requests with filter rules',
    declarativeNetRequestWithHostAccess: 'Modify network requests on permitted sites',
    scripting: 'Inject scripts and styles into matching pages',
    cookies: 'Read and modify site cookies',
    contextMenus: 'Add items to the browser context menu',
    alarms: 'Schedule background tasks',
    unlimitedStorage: 'Store large amounts of local extension data'
  };
  if (map[perm]) return map[perm];
  if (perm === '<all_urls>' || perm === '*://*/*') return 'Read and change data on all websites';
  if (perm.includes('://')) return `Access site data on ${perm}`;
  return `Use the "${perm}" extension API`;
}

function renderExtensionsDashboard(view, signal) {
  view.innerHTML = `<header class="page-header"><div><h1>Extensions</h1>
    <p>Install Chrome Web Store packages, .crx/.zip archives, or unpacked Manifest V3 folders into a workspace after reviewing permissions.</p></div>
    <a class="btn primary" href="https://chromewebstore.google.com/">Browse Chrome Web Store</a></header>
    <section class="panel">
      <form class="field-grid" id="ext-review-form">
        <div class="field-stack">
          <label for="ext-folder-path">Store URL, extension ID, .crx/.zip, or unpacked folder</label>
          <div class="field-action">
            <input class="input" id="ext-folder-path" name="path" type="text" placeholder="https://chromewebstore.google.com/detail/... or C:\\path\\to\\ext" aria-describedby="ext-review-status" required>
            <button class="btn primary" type="submit">Review extension</button>
          </div>
        </div>
        <label class="field-stack">Workspace<span class="select-wrap"><select class="input" name="workspace" id="ext-workspace">
          <option value="Work">Work</option><option value="Personal">Personal</option>
        </select></span></label>
      </form>
      <p class="status" id="ext-review-status" role="status"></p>
    </section>
    <section class="panel ext-review-card" id="ext-review-panel" hidden></section>
    <section class="settings-section" aria-labelledby="installed-extensions-heading">
      <h2 id="installed-extensions-heading">Installed extensions</h2>
      <p class="status" id="ext-list-status" role="status">Loading extensions…</p>
      <div class="ext-grid" id="installed-extensions"></div>
    </section>`;

  const reviewForm = view.querySelector('#ext-review-form');
  const reviewStatus = view.querySelector('#ext-review-status');
  const reviewPanel = view.querySelector('#ext-review-panel');
  const listStatus = view.querySelector('#ext-list-status');
  const listEl = view.querySelector('#installed-extensions');

  async function load() {
    const data = await callWorkspace('state');
    if (signal.aborted) return;
    if (data.workspace) view.querySelector('#ext-workspace').value = data.workspace;
    const exts = data.extensions || [];
    listEl.innerHTML = exts.map(ext => {
      const initial = (ext.name || '?').trim().charAt(0).toUpperCase();
      const perms = ext.permissions || [];
      const hosts = (ext.host_permissions || []).length
        ? ext.host_permissions
        : (ext.content_scripts || []).flatMap(c => c.matches || []);
      const chips = [...perms.slice(0, 4), ...hosts.slice(0, 2)];
      return `<article class="ext-card">
        <div class="ext-card-head">
          <div class="ext-avatar" aria-hidden="true">${escapeText(initial)}</div>
          <div class="ext-meta">
            <h3>${escapeText(ext.name)}</h3>
            <div class="ext-sub">
              <span class="ext-pill">v${escapeText(ext.version)}</span>
              <span class="ext-pill">${escapeText(ext.workspace)}</span>
              <span class="ext-pill ${ext.enabled ? 'on' : ''}">${ext.enabled ? 'Enabled' : 'Disabled'}</span>
            </div>
          </div>
          ${iconToggle('enabled', (ext.enabled ? 'Disable ' : 'Enable ') + ext.name, ext.enabled, ext.id)}
        </div>
        <div class="ext-perms">
          ${chips.length
            ? chips.map(p => `<span class="ext-pill">${escapeText(p)}</span>`).join('')
            : '<span class="ext-pill">No special permissions</span>'}
          ${(perms.length + hosts.length) > chips.length ? `<span class="ext-pill">+${perms.length + hosts.length - chips.length} more</span>` : ''}
        </div>
        <div class="ext-card-foot">
          <div class="row compact">
            ${ext.options_path ? `<a class="btn" href="harbor://extension-asset/${encodeURIComponent(ext.id)}/${encodeURIComponent(ext.version)}/${encodeURIComponent(ext.options_path)}">Options</a>` : ''}
          </div>
          <button class="btn danger" type="button" data-action="remove" data-id="${escapeText(ext.id)}">${icon('delete')}<span>Remove</span></button>
        </div>
      </article>`;
    }).join('') || '<p class="empty-state">No extensions installed.</p>';

    listEl.querySelectorAll('.ext-card').forEach((card, index) => {
      const ext = exts[index];
      const toggle = card.querySelector('[data-action=enabled]');
      const remove = card.querySelector('[data-action=remove]');
      toggle.onclick = runAction(toggle, listStatus, async () => {
        const enabled = toggle.getAttribute('aria-pressed') !== 'true';
        await callWorkspace('set_extension_enabled', { id: ext.id, enabled });
        toggle.setAttribute('aria-pressed', String(enabled));
        setStatus(listStatus, enabled ? 'Extension enabled.' : 'Extension disabled.');
        await load();
      });
      remove.onclick = runAction(remove, listStatus, async () => {
        if (await confirmDashboard('Remove extension?', `${ext.name} will be uninstalled from ${ext.workspace}.`, 'Remove extension')) {
          await callWorkspace('remove_extension', { id: ext.id });
          setStatus(listStatus, 'Extension removed.');
          await load();
        } else setStatus(listStatus, '');
      });
    });
    setStatus(listStatus, '');
  }

  reviewForm.onsubmit = runAction(reviewForm.querySelector('button'), reviewStatus, async () => {
    const rev = await callWorkspace('review_extension', {
      path: reviewForm.elements.path.value,
      workspace: reviewForm.elements.workspace.value
    });
    if (signal.aborted) return;
    const requested = [...(rev.permissions || []), ...(rev.host_permissions || [])];
    reviewPanel.hidden = false;
    reviewPanel.innerHTML = `<div class="ext-card-head">
        <div class="ext-avatar" aria-hidden="true">${escapeText((rev.name || '?').trim().charAt(0).toUpperCase())}</div>
        <div class="ext-meta">
          <h2>${escapeText(rev.name)} <small class="subtle">v${escapeText(rev.version)}</small></h2>
          <div class="ext-sub">
            <span class="ext-pill">${escapeText(rev.workspace)}</span>
            <span class="ext-pill">${escapeText(String(rev.content_scripts_count))} content script(s)</span>
          </div>
        </div>
      </div>
      ${rev.description ? `<p>${escapeText(rev.description)}</p>` : ''}
      <h3>Requested permissions</h3>
      <ul class="ext-perm-list">
        ${requested.length
          ? requested.map(p => `<li class="ext-perm-item"><div><strong>${escapeText(p)}</strong><small>${escapeText(describeExtensionPermission(p))}</small></div></li>`).join('')
          : '<li class="ext-perm-item"><div><strong>No special permissions</strong><small>Only declared content script rules apply</small></div></li>'}
      </ul>
      <dl class="review-details">
        <div><dt>Source</dt><dd>${escapeText(rev.source_path)}</dd></div>
        <div><dt>Package SHA-256</dt><dd><code>${escapeText(rev.digest)}</code></dd></div>
      </dl>
      ${(rev.compatibility_notes && rev.compatibility_notes.length) ? `<div class="status" data-error>${rev.compatibility_notes.map(n => escapeText(n)).join('<br>')}</div>` : ''}
      <div class="row wrap compact">
        <button class="btn primary" type="button" id="ext-install-btn">Accept &amp; Install</button>
        <button class="btn danger" type="button" id="ext-decline-btn">Reject</button>
      </div>`;
    reviewPanel.querySelector('#ext-decline-btn').onclick = () => {
      reviewPanel.hidden = true;
      reviewPanel.innerHTML = '';
      setStatus(reviewStatus, 'Extension rejected.');
    };
    const installBtn = reviewPanel.querySelector('#ext-install-btn');
    installBtn.onclick = runAction(installBtn, reviewStatus, async () => {
      await callWorkspace('install_extension', { token: rev.token });
      reviewPanel.hidden = true;
      reviewPanel.innerHTML = '';
      reviewForm.elements.path.value = '';
      setStatus(reviewStatus, 'Extension installed.');
      await load();
    });
    setStatus(reviewStatus, 'Review permissions before accepting or rejecting.');
  });

  load().then(() => {
    const installParam = new URLSearchParams(location.search).get('install');
    if (installParam && !signal.aborted) {
      history.replaceState(null, '', 'harbor://extensions');
      reviewForm.elements.path.value = installParam;
      reviewForm.requestSubmit();
    }
  }).catch(error => setStatus(listStatus, error.message, true));
}

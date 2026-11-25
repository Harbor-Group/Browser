'use strict';

const icons = {
  brand: '<rect x="1" y="1" width="22" height="22" rx="5" fill="#1e293b" stroke="#394657" stroke-width="1"/><circle cx="12" cy="12" r="8.2" stroke="#e2e8f0" stroke-width="1.35" stroke-dasharray="9 4.5" stroke-dashoffset="2"/><circle cx="12" cy="12" r="5.9" stroke="#62d0fc" stroke-width="1.35" stroke-dasharray="7 3.8" stroke-dashoffset="6"/><polygon points="12,7.6 15.1,8.9 16.4,12 15.1,15.1 12,16.4 8.9,15.1 7.6,12 8.9,8.9" fill="#1e293b" stroke="#62d0fc" stroke-width="1.15"/><path d="M12 7.6v8.8M7.6 12h8.8M8.9 8.9l6.2 6.2M15.1 8.9l-6.2 6.2" stroke="#e2e8f0" stroke-width="1.1"/><circle cx="12" cy="12" r="1.4" fill="#62d0fc" stroke="none"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  home: '<path d="m3 10 9-7 9 7v10H3z"/><path class="toggle-off-slash" d="M3 3l18 18"/>',
  'bg-tabs': '<rect x="3" y="7" width="13" height="12" rx="2"/><path d="M7 7V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-2"/><path class="toggle-off-slash" d="M3 3l18 18"/>',
  'bg-audio': '<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path class="toggle-on-wave" d="M15.5 8.5a5 5 0 0 1 0 7"/><path class="toggle-off-slash" d="M3 3l18 18"/>',
  bookmarks: '<path d="M6 4h12v16l-6-4-6 4z"/>',
  history: '<path d="M3 12a9 9 0 1 0 3-7l-3 3m0-5v5h5m4-1v5l3 2"/>',
  logins: '<circle cx="8" cy="15" r="4"/><path d="m11 12 8-8 2 2-2 2 1 1-3 3"/>',
  shield: '<path d="M12 22s7-4 7-10V5l-7-3-7 3v7c0 6 7 10 7 10Z"/><path d="m9 12 2 2 4-4"/>',
  scripts: '<path d="m7 6-5 6 5 6m10-12 5 6-5 6m-3-15-4 18"/>',
  extensions: '<path d="M14 6a2 2 0 1 0-4 0H5v5a2 2 0 1 1 0 4v5h5a2 2 0 1 1 4 0h5v-5a2 2 0 1 0 0-4V6z"/>',
  server: '<rect x="4" y="4" width="16" height="7" rx="2"/><rect x="4" y="13" width="16" height="7" rx="2"/><path d="M8 7.5h1m-1 9h1"/>',
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="2.5" fill="var(--side)"/><circle cx="15" cy="17" r="2.5" fill="var(--side)"/>',
  move: '<path d="M3 7V5h7l2 2h9v12H3V7m4 6h9m-3-3 3 3-3 3"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  reveal: '<path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6Z"/><circle cx="12" cy="12" r="2.5"/>',
  enabled: '<rect class="toggle-shape" x="5" y="5" width="14" height="14" rx="4"/><path class="toggle-mark" d="m8 12 3 3 5-6"/>',
  edit: '<path d="m4 16 12-12 4 4L8 20H4z M14 6l4 4"/>',
  delete: '<path d="M4 7h16m-14 0 1 13h10l1-13M9 7V4h6v3m-5 3v7m4-7v7"/>'
};

function icon(name) {
  return `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">${icons[name]}</svg>`;
}

function actionButton(action, label, id) {
  return `<button class="btn icon-control ${action === 'delete' ? 'danger' : ''}" type="button" data-action="${action}" data-id="${escapeText(id)}" aria-label="${escapeText(label)}" title="${escapeText(label)}">
    ${icon(action)}</button>`;
}

function iconToggle(name, label, pressed, id = '') {
  return `<button class="btn icon-control icon-toggle" type="button" data-action="${name}" data-id="${escapeText(id)}"
    aria-label="${escapeText(label)}" title="${escapeText(label)}" aria-pressed="${pressed}">${icon(name)}</button>`;
}

function restoreRowFocus(container, deletedIndex, selector, fallback) {
  const controls = container.querySelectorAll(selector);
  (controls[Math.min(deletedIndex, controls.length - 1)] || fallback).focus();
}

function confirmDashboard(title, detail, action) {
  return new Promise(resolve => {
    const dialog = document.createElement('dialog');
    dialog.setAttribute('aria-labelledby', 'confirm-title');
    dialog.setAttribute('aria-describedby', 'confirm-detail');
    dialog.innerHTML = `<h2 id="confirm-title">${escapeText(title)}</h2><p id="confirm-detail">${escapeText(detail)}</p>
      <div class="row wrap compact"><button class="btn" type="button" autofocus>Cancel</button>
      <button class="btn danger" type="button">${escapeText(action)}</button></div>`;
    const buttons = dialog.querySelectorAll('button');
    buttons[0].onclick = () => dialog.close();
    buttons[1].onclick = () => dialog.close('confirm');
    dialog.onclose = () => {
      resolve(dialog.returnValue === 'confirm');
      dialog.remove();
    };
    document.body.append(dialog);
    dialog.showModal();
  });
}

function renderHomeDashboard(view) {
  const cards = [
    ['bookmarks', 'Bookmarks', 'Saved pages', 'bookmarks'],
    ['history', 'History', 'Recorded visits', 'history'],
    ['scripts', 'User scripts', 'Installed scripts', 'scripts'],
    ['logins', 'Logins', 'Saved credentials', 'logins'],
    ['lists', 'Filter lists', 'Subscribed sources', 'adblock'],
    ['blocked', 'Blocked requests', 'This session', 'adblock']
  ];
  view.innerHTML = `<header class="page-header"><h1>Personal workspace</h1></header>
    <h2>Workspace activity</h2>
    <div class="grid">${cards.map(([key, title, description, route]) => `
      <button class="card" data-page-link="${route}">
        <span class="card-heading">${icon(route === 'adblock' ? 'shield' : route)}<strong>${title}</strong></span>
        <span class="stat" data-stat="${key}">—</span><span class="subtle">${description}</span>
      </button>`).join('')}</div>
    <section class="panel sync-card"><div><h2>Home Sync Server</h2><p id="home-sync-status" role="status">Checking sync status…</p></div>
      <button class="btn" id="home-server-link">Manage sync</button></section>
    <p class="status" id="home-stats-status" role="status"></p>`;
  const syncStatus = view.querySelector('#home-sync-status');
  const syncLink = view.querySelector('#home-server-link');
  const status = view.querySelector('#home-stats-status');
  view.querySelectorAll('[data-page-link]').forEach(button => {
    button.onclick = () => setRoute(button.dataset.pageLink);
  });
  syncLink.onclick = () => setRoute('server');
  callWorkspace('state').then(stats => {
    view.querySelectorAll('[data-stat]').forEach(node => {
      node.textContent = new Intl.NumberFormat().format(stats[node.dataset.stat]);
    });
  }).catch(error => setStatus(status, error.message, true));
  callWorkspace('sync_status').then(sync => {
    if (sync.error) setStatus(syncStatus, sync.error, true);
    else if (sync.last_sync && sync.unlocked) setStatus(syncStatus, 'Last synced ' + new Date(sync.last_sync * 1000).toLocaleString());
    else setStatus(syncStatus, sync.unlocked ? 'Automatic sync is on.' : sync.paired ? 'Unlock your vault to resume sync.' : 'Stored on this device.');
    if (!sync.unlocked) syncLink.textContent = sync.paired ? 'Unlock vault' : 'Connect server';
  }).catch(() => setStatus(syncStatus, 'Sync status is unavailable.', true));
}

function renderBookmarksDashboard(view) {
  view.innerHTML = `<header class="page-header"><h1>Bookmarks</h1>
      <button class="btn primary" id="new-bookmark">Add bookmark</button></header>
    <div class="field-grid">
      <label class="field-stack">Search bookmarks<input class="input" type="search" id="bookmark-search" placeholder="Search by title or address"></label>
      <form class="field-stack" id="new-folder-form">
        <label for="new-folder-name">New folder</label>
        <div class="field-action"><input class="input" id="new-folder-name" name="name" placeholder="Folder name" maxlength="120" aria-describedby="bookmark-status" required>
          <button class="btn" type="submit">Create folder</button></div>
      </form>
    </div>
    <p class="status" id="bookmark-status" role="status">Loading…</p><div id="folders"></div>
    <dialog id="bookmark-editor" aria-labelledby="bookmark-editor-title"><form class="settings-form">
      <h2 id="bookmark-editor-title">Add bookmark</h2>
      <label>Title<input class="input" name="title"></label>
      <label>Address<input class="input" name="url" required></label>
      <label>Folder<span class="select-wrap"><select class="input" name="folder"></select></span></label>
      <p class="status" role="status"></p>
      <div class="row"><button class="btn primary" type="submit">Save</button><button class="btn" type="button" data-cancel>Cancel</button></div>
    </form></dialog>
    <dialog id="folder-editor" aria-labelledby="folder-editor-title"><form class="settings-form">
      <h2 id="folder-editor-title">Rename folder</h2>
      <label>Name<input class="input" name="name" required maxlength="120"></label>
      <p class="status" role="status"></p>
      <div class="row"><button class="btn primary" type="submit">Save</button><button class="btn" type="button" data-cancel>Cancel</button></div>
    </form></dialog>
    <dialog id="bookmark-move" aria-labelledby="bookmark-move-title"><form class="settings-form">
      <h2 id="bookmark-move-title">Move bookmark</h2>
      <label>Destination folder<span class="select-wrap"><select class="input" name="folder"></select></span></label>
      <p class="status" role="status"></p>
      <div class="row"><button class="btn primary" type="submit">Move bookmark</button><button class="btn" type="button" data-cancel>Cancel</button></div>
    </form></dialog>`;
  const search = view.querySelector('#bookmark-search');
  const folders = view.querySelector('#folders');
  const status = view.querySelector('#bookmark-status');
  const bookmarkEditor = view.querySelector('#bookmark-editor');
  const folderEditor = view.querySelector('#folder-editor');
  const moveDialog = view.querySelector('#bookmark-move');
  const newFolderForm = view.querySelector('#new-folder-form');
  const bookmarkForm = bookmarkEditor.querySelector('form');
  const folderForm = folderEditor.querySelector('form');
  const moveForm = moveDialog.querySelector('form');
  let library = { bookmarks: [], folders: [] };
  let editingBookmark = null;
  let editingFolder = null;
  let movingBookmark;
  let moving = false;

  function fillFolders(select, bookmark) {
    select.replaceChildren(new Option('Unfiled', ''));
    library.folders.forEach(folder => select.add(new Option(folder.name, folder.id)));
    select.value = library.folders.some(folder => folder.id === bookmark?.folder) ? bookmark.folder : '';
  }

  function openBookmark(bookmark) {
    editingBookmark = bookmark?.id ?? null;
    bookmarkForm.querySelector('h2').textContent = bookmark ? 'Edit bookmark' : 'Add bookmark';
    bookmarkForm.elements.title.value = bookmark?.title ?? '';
    bookmarkForm.elements.url.value = bookmark?.url ?? '';
    fillFolders(bookmarkForm.elements.folder, bookmark);
    setStatus(bookmarkForm.querySelector('[role=status]'), '');
    bookmarkEditor.showModal();
  }

  function openFolder(folder) {
    editingFolder = folder.id;
    folderForm.elements.name.value = folder.name;
    setStatus(folderForm.querySelector('[role=status]'), '');
    folderEditor.showModal();
  }

  function draw() {
    const query = search.value.trim().toLowerCase();
    const named = new Set(library.folders.map(folder => folder.id));
    folders.innerHTML = [{ id: '', name: 'Unfiled' }, ...library.folders].map(folder => {
      const rows = library.bookmarks.filter(item => {
        const id = named.has(item.folder) ? item.folder : '';
        return id === folder.id && (item.title + ' ' + item.url).toLowerCase().includes(query);
      });
      return `<section class="folder" data-folder="${escapeText(folder.id)}">
        <div class="toolbar folder-header"><h2 class="compact">${escapeText(folder.name)}</h2>
        ${folder.id ? `<div class="actions">${actionButton('edit', 'Rename folder', folder.id)}${actionButton('delete', 'Delete folder', folder.id)}</div>` : ''}</div>
        ${rows.map(item => `<div class="listitem" draggable="true" data-bookmark="${escapeText(item.id)}">
          <div class="bookmark-name"><button class="link-button" data-open>${escapeText(item.title)}</button><small>${escapeText(item.url)}</small></div>
          <div class="actions">${actionButton('move', 'Move bookmark', item.id)}${actionButton('edit', 'Edit bookmark', item.id)}${actionButton('delete', 'Delete bookmark', item.id)}</div>
        </div>`).join('') || '<p class="empty-state">No bookmarks here.</p>'}</section>`;
    }).join('');

    folders.querySelectorAll('[data-folder]').forEach((section, index) => {
      const selected = library.folders.find(folder => folder.id === section.dataset.folder);
      const header = section.querySelector('.folder-header');
      if (selected) {
        header.querySelector('[data-action=edit]').onclick = () => openFolder(selected);
        const remove = header.querySelector('[data-action=delete]');
        remove.onclick = runAction(remove, status, async () => {
          if (await confirmDashboard('Delete folder?', 'Its bookmarks will move to Unfiled.', 'Delete folder')) {
            await callWorkspace('delete_folder', { id: selected.id });
            await load();
            restoreRowFocus(folders, index - 1, '.folder-header [data-action=delete]', newFolderForm.elements.name);
            setStatus(status, 'Folder deleted.');
          } else setStatus(status, '');
        });
      }
      section.ondragover = event => {
        if (!event.dataTransfer.types.includes('application/x-harbor-bookmark')) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'move';
        section.classList.add('drop-target');
      };
      section.ondragleave = event => {
        if (!section.contains(event.relatedTarget)) section.classList.remove('drop-target');
      };
      section.ondrop = async event => {
        event.preventDefault();
        section.classList.remove('drop-target');
        const id = event.dataTransfer.getData('application/x-harbor-bookmark');
        if (!library.bookmarks.some(item => item.id === id)) return;
        try {
          await moveBookmark(id, selected?.id ?? null);
        } catch (error) {
          setStatus(status, error.message, true);
        }
      };
    });
    folders.querySelectorAll('[data-bookmark]').forEach((row, index) => {
      const bookmark = library.bookmarks.find(item => item.id === row.dataset.bookmark);
      row.querySelector('[data-open]').onclick = () => openSavedAddress(bookmark.url);
      row.querySelector('[data-action=edit]').onclick = () => openBookmark(bookmark);
      row.querySelector('[data-action=move]').onclick = () => {
        movingBookmark = bookmark.id;
        fillFolders(moveForm.elements.folder, bookmark);
        setStatus(moveForm.querySelector('[role=status]'), '');
        moveDialog.showModal();
      };
      row.ondragstart = event => {
        event.dataTransfer.setData('application/x-harbor-bookmark', bookmark.id);
        event.dataTransfer.effectAllowed = 'move';
      };
      const remove = row.querySelector('[data-action=delete]');
      remove.onclick = runAction(remove, status, async () => {
        if (await confirmDashboard('Delete bookmark?', bookmark.title, 'Delete bookmark')) {
          await callWorkspace('delete_record', { id: bookmark.id });
          await load();
          restoreRowFocus(folders, index, '[data-bookmark] [data-open]', search);
          setStatus(status, 'Bookmark deleted.');
        } else setStatus(status, '');
      });
    });
  }

  async function load() {
    library = await callWorkspace('state');
    draw();
    setStatus(status, '');
  }
  async function moveBookmark(id, folder) {
    if (moving) throw new Error('A bookmark is already being moved.');
    moving = true;
    try {
      await callWorkspace('move_bookmark', { id, folder });
      await load();
      setStatus(status, 'Bookmark moved.');
    } finally {
      moving = false;
    }
  }

  newFolderForm.onsubmit = runAction(newFolderForm.querySelector('[type=submit]'), status, async () => {
    const field = newFolderForm.elements.name;
    field.readOnly = true;
    try {
      await callWorkspace('save_folder', { id: null, name: field.value });
      newFolderForm.reset();
      await load();
      setStatus(status, 'Folder created.');
    } finally {
      field.readOnly = false;
    }
  });
  view.querySelector('#new-bookmark').onclick = () => openBookmark();
  search.oninput = draw;
  for (const dialog of [folderEditor, bookmarkEditor, moveDialog]) dialog.querySelector('[data-cancel]').onclick = () => dialog.close();
  bookmarkForm.onsubmit = runAction(bookmarkForm.querySelector('[type=submit]'), bookmarkForm.querySelector('[role=status]'), async () => {
    const id = editingBookmark;
    const fields = Object.fromEntries(new FormData(bookmarkForm));
    await callWorkspace('save_bookmark', { ...fields, id, folder: fields.folder || null });
    bookmarkEditor.close();
    await load();
    const row = [...folders.querySelectorAll('[data-bookmark]')].find(bookmark => bookmark.dataset.bookmark === id);
    (row?.querySelector('[data-action=edit]') || view.querySelector('#new-bookmark')).focus();
    setStatus(status, 'Bookmark saved.');
  });
  moveForm.onsubmit = runAction(moveForm.querySelector('[type=submit]'), moveForm.querySelector('[role=status]'), async () => {
    const id = movingBookmark;
    await moveBookmark(id, moveForm.elements.folder.value || null);
    moveDialog.close();
    const moved = [...folders.querySelectorAll('[data-bookmark]')].find(row => row.dataset.bookmark === id);
    (moved?.querySelector('[data-action=move]') || search).focus();
  });
  folderForm.onsubmit = runAction(folderForm.querySelector('[type=submit]'), folderForm.querySelector('[role=status]'), async () => {
    const id = editingFolder;
    await callWorkspace('save_folder', { id, name: folderForm.elements.name.value });
    folderEditor.close();
    await load();
    const folder = [...folders.querySelectorAll('[data-folder]')].find(section => section.dataset.folder === id);
    (folder?.querySelector('[data-action=edit]') || newFolderForm.elements.name).focus();
    setStatus(status, 'Folder renamed.');
  });
  load().catch(error => setStatus(status, error.message, true));
}



function openSavedAddress(raw) {
  const url = new URL(raw);
  if (['http:', 'https:', 'harbor:'].includes(url.protocol)) location.href = url.href;
}

function renderHistoryDashboard(view, signal) {
  view.innerHTML = `<header class="page-header"><h1>History</h1>
      <details class="history-clear" id="history-clear">
        <summary class="btn danger">Clear history<span class="dropdown-arrow" aria-hidden="true"></span></summary>
        <div class="history-ranges">
          <button type="button" data-range="week">Last 7 days</button>
          <button type="button" data-range="month">Last month</button>
          <button type="button" data-range="year">Last year</button>
          <button type="button" data-range="all">All time</button>
        </div>
      </details></header>
    <label class="field-stack">Search history<input class="input" type="search" id="history-search" placeholder="Search by title or address"></label>
    <p class="status" id="history-status" role="status">Loading…</p><div id="history-items"></div>`;
  const status = view.querySelector('#history-status');
  const search = view.querySelector('#history-search');
  const list = view.querySelector('#history-items');
  const clear = view.querySelector('#history-clear');
  const trigger = clear.querySelector('summary');
  const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'full' });
  const timeFormat = new Intl.DateTimeFormat(undefined, { timeStyle: 'short' });
  let items = [];
  const visitTime = item => new Date(item.visited_at * 1000).getTime();

  function draw() {
    const query = search.value.trim().toLowerCase();
    const results = items.filter(item => (item.title + ' ' + item.url).toLowerCase().includes(query));
    const groups = new Map();
    const visits = new Map(results.map(item => [item.id, item]));
    for (const item of results) {
      const date = Number.isFinite(visitTime(item)) ? dateFormat.format(visitTime(item)) : 'Unknown date';
      if (!groups.has(date)) groups.set(date, []);
      groups.get(date).push(item);
    }
    list.innerHTML = [...groups].map(([date, entries]) => `<section class="history-group"><h2>${escapeText(date)}</h2>
      ${entries.map(item => `<div class="listitem" data-id="${escapeText(item.id)}">
      <div class="bookmark-name"><button class="link-button" data-open>${escapeText(item.title || item.url)}</button>
        <small>${escapeText(item.url)}</small></div>
      <div class="actions"><span class="history-time">${Number.isFinite(visitTime(item)) ? escapeText(timeFormat.format(visitTime(item))) : ''}</span>
      ${actionButton('delete', 'Remove from history', item.id)}</div></div>`).join('')}</section>`).join('') ||
      `<p class="empty-state">${query ? 'No matching visits.' : 'No visits to show.'}</p>`;
    list.querySelectorAll('.listitem').forEach((row, index) => {
      const item = visits.get(row.dataset.id);
      row.querySelector('[data-open]').onclick = () => openSavedAddress(item.url);
      const remove = row.querySelector('[data-action=delete]');
      remove.onclick = runAction(remove, status, async () => {
        await callWorkspace('delete_record', { id: item.id });
        await load();
        restoreRowFocus(list, index, '[data-open]', search);
        setStatus(status, 'Visit removed.');
      });
    });
  }
  async function load() {
    items = await callWorkspace('state');
    draw();
    setStatus(status, '');
  }
  clear.addEventListener('keydown', event => {
    if (event.key === 'Escape' && clear.open) {
      event.preventDefault();
      clear.open = false;
      trigger.focus();
    }
  });
  document.addEventListener('pointerdown', event => {
    if (clear.open && !clear.contains(event.target)) {
      if (clear.contains(document.activeElement)) trigger.focus();
      clear.open = false;
    }
  }, { signal });
  clear.querySelectorAll('[data-range]').forEach(button => {
    button.onclick = runAction(button, status, async () => {
      clear.open = false;
      trigger.focus();
      const range = button.dataset.range;
      const title = range === 'all' ? 'Clear all history?' : `Clear history for the ${button.textContent.toLowerCase()}?`;
      const confirmed = await confirmDashboard(title, 'Selected visits will be removed from your Harbor history.', 'Clear history');
      if (!confirmed) { setStatus(status, ''); return; }
      const now = new Date();
      const from = historyRangeStart(range, now);
      const until = from === null ? null : Math.floor(now.getTime() / 1000) + 1;
      const result = await callWorkspace('clear_history', { from, until });
      await load();
      setStatus(status, `${result.removed} ${result.removed === 1 ? 'visit' : 'visits'} removed.`);
    });
  });
  search.oninput = draw;
  load().catch(error => setStatus(status, error.message, true));
}



function historyRangeStart(range, now) {
  if (range === 'all') return null;
  const start = new Date(now);
  if (range === 'week') {
    start.setDate(start.getDate() - 7);
  } else {
    const day = start.getDate();
    start.setDate(1);
    start.setMonth(start.getMonth() - (range === 'year' ? 12 : 1));
    const lastDay = new Date(start.getFullYear(), start.getMonth() + 1, 0).getDate();
    start.setDate(Math.min(day, lastDay));
  }
  return Math.floor(start.getTime() / 1000);
}

function renderAdblockDashboard(view) {
  view.innerHTML = `<header class="page-header"><h1>Adblocker</h1></header>
    <section class="settings-section" aria-labelledby="subscription-heading">
      <h2 id="subscription-heading">Subscribed lists</h2>
      <form class="field-grid" id="adblock-subscribe">
        <label class="field-stack">List preset<span class="select-wrap"><select class="input" name="preset">
          <option value="">Custom URL</option>
          <option value="https://easylist.to/easylist/easylist.txt">EasyList</option>
          <option value="https://easylist.to/easylist/easyprivacy.txt">EasyPrivacy</option>
        </select></span></label>
        <div class="field-stack"><label for="filter-url">Filter list URL</label>
          <div class="field-action"><input class="input" id="filter-url" name="url" type="url" placeholder="https://example.com/filters.txt" aria-describedby="adblock-status" required>
            <button class="btn primary" type="submit" disabled>Add list</button></div></div>
      </form>
      <p class="status" id="adblock-status" role="status">Loading lists…</p>
      <div id="subscriptions"></div>
    </section>
    <section class="settings-section" aria-labelledby="custom-filters-heading">
      <h2 id="custom-filters-heading">Custom rules</h2>
      <label class="field-stack">Filter rules, one per line
        <textarea class="textarea" id="custom-filters" placeholder="||tracker.example^" spellcheck="false" disabled></textarea></label>
      <p class="status" id="rules-status" role="status"></p>
      <div class="row wrap compact"><button class="btn primary" id="save-rules" disabled>Save custom list</button>
        <button class="btn danger" id="clear-rules" disabled>Remove custom list</button></div>
    </section>`;
  const status = view.querySelector('#adblock-status');
  const rulesStatus = view.querySelector('#rules-status');
  const form = view.querySelector('#adblock-subscribe');
  const subscribe = form.querySelector('button');
  const save = view.querySelector('#save-rules');
  const clear = view.querySelector('#clear-rules');
  const editor = view.querySelector('#custom-filters');
  const subscriptions = view.querySelector('#subscriptions');
  let config = { lists: [], rules: [] };
  let busy = false;

  function draw() {
    subscriptions.innerHTML = config.lists.map((url, index) => {
      const preset = [...form.elements.preset.options].find(option => option.value === url);
      const name = preset?.text || (URL.canParse(url) ? new URL(url).hostname : url);
      return `<div class="listitem"><div class="break"><strong>${escapeText(name)}</strong><small>Subscribed · ${escapeText(url)}</small></div>
        ${actionButton('delete', 'Remove subscription', index)}</div>`;
    }).join('') || '<p class="empty-state">No subscribed filter lists.</p>';
    subscriptions.querySelectorAll('[data-action=delete]').forEach((button, index) => {
      button.onclick = runAction(button, status, async () => {
        await saveConfig({ ...config, lists: config.lists.filter((_, row) => row !== index) }, status, 'Subscription removed.');
        restoreRowFocus(subscriptions, index, '[data-action=delete]', form.elements.url);
      });
    });
  }

  async function saveConfig(next, feedback, message) {
    if (busy) throw new Error('A filter change is already being saved.');
    busy = true;
    try {
      await callWorkspace('save_adblock', next);
      config = next;
      draw();
      setStatus(feedback, message);
    } finally {
      busy = false;
    }
  }
  form.elements.preset.onchange = () => {
    form.elements.url.value = form.elements.preset.value;
    if (!form.elements.preset.value) form.elements.url.focus();
  };
  form.onsubmit = runAction(subscribe, status, async () => {
    const { url: field, preset } = form.elements;
    const url = field.value.trim();
    if (!url) throw new Error('Choose a list or enter an HTTPS filter-list URL.');
    field.readOnly = preset.disabled = true;
    try {
      await saveConfig({ ...config, lists: [...new Set([...config.lists, url])] }, status, 'Subscription saved.');
      form.reset();
    } finally {
      field.readOnly = preset.disabled = false;
    }
  });
  save.onclick = runAction(save, rulesStatus, () => saveConfig({
    ...config, rules: editor.value.split('\n').map(rule => rule.trim()).filter(Boolean)
  }, rulesStatus, 'Custom list saved.'));
  clear.onclick = runAction(clear, rulesStatus, async () => {
    if (await confirmDashboard('Remove the custom list?', 'Subscribed filter lists will stay enabled.', 'Remove custom list')) {
      await saveConfig({ ...config, rules: [] }, rulesStatus, 'Custom list removed.');
      editor.value = '';
    } else setStatus(rulesStatus, '');
  });
  callWorkspace('state').then(result => {
    config = result;
    editor.value = config.rules.join('\n');
    draw();
    subscribe.disabled = save.disabled = clear.disabled = editor.disabled = false;
    setStatus(status, '');
  }).catch(error => setStatus(status, error.message, true));
}

function renderLoginsDashboard(view) {
  view.innerHTML = `<header class="page-header"><div><h1>Saved logins</h1>
    <p>Saved passwords are encrypted on this device and when synced.</p></div>
    <button class="btn primary" id="new-login">Add login</button></header>
    <p class="status" id="login-status" role="status">Loading logins…</p>
    <div id="saved-logins"></div>
    <dialog id="login-edit" aria-labelledby="login-edit-title"><form class="settings-form">
      <h2 id="login-edit-title">Add login</h2>
      <label>Website<input class="input" type="url" name="origin" placeholder="https://example.com" required></label>
      <label>Username or email<input class="input" name="username" autocomplete="off"></label>
      <label>Password<input class="input" type="password" name="password" autocomplete="new-password" aria-describedby="login-password-help"></label>
      <small class="subtle" id="login-password-help"></small>
      <p class="status" role="status"></p>
      <div class="row"><button class="btn primary" type="submit">Save login</button><button class="btn" type="button" data-close>Cancel</button></div>
    </form></dialog>
    <dialog id="login-reveal" aria-labelledby="login-reveal-title">
      <h2 id="login-reveal-title">Password</h2>
      <p class="credentials break" id="revealed-password"></p>
      <button class="btn" type="button" data-close>Hide password</button>
    </dialog>`;
  const status = view.querySelector('#login-status');
  const list = view.querySelector('#saved-logins');
  const add = view.querySelector('#new-login');
  const editor = view.querySelector('#login-edit');
  const reveal = view.querySelector('#login-reveal');
  const revealed = reveal.querySelector('#revealed-password');
  const form = editor.querySelector('form');
  const save = form.querySelector('[type=submit]');
  const formStatus = form.querySelector('[role=status]');
  let editing;

  function openEditor(login) {
    if (save.disabled) return;
    editing = login?.id ?? null;
    form.querySelector('h2').textContent = editing ? 'Edit login' : 'Add login';
    form.elements.origin.value = login?.origin ?? '';
    form.elements.username.value = login?.username ?? '';
    form.elements.password.value = '';
    form.elements.password.required = !editing;
    form.querySelector('#login-password-help').textContent = editing ? 'Leave blank to keep the current password.' : '';
    setStatus(formStatus, '');
    editor.showModal();
  }

  async function load() {
    const logins = await callWorkspace('state');
    list.innerHTML = logins.map(login => `<div class="listitem login-row">
      <div class="break"><strong>${escapeText(login.origin)}</strong><small>${escapeText(login.username || 'No username')}</small></div>
      <div class="actions">${actionButton('reveal', 'Show password', login.id)}${actionButton('copy', 'Copy password', login.id)}
      ${actionButton('edit', 'Edit login', login.id)}${actionButton('delete', 'Delete login', login.id)}</div>
    </div>`).join('') || '<p class="empty-state">No saved logins.</p>';
    list.querySelectorAll('.login-row').forEach((row, index) => {
      const login = logins[index];
      const show = row.querySelector('[data-action=reveal]');
      const copy = row.querySelector('[data-action=copy]');
      const remove = row.querySelector('[data-action=delete]');
      row.querySelector('[data-action=edit]').onclick = () => openEditor(login);
      show.onclick = runAction(show, status, async () => {
        const { password } = await callWorkspace('login_password', { id: login.id });
        if (reveal.open) return;
        revealed.textContent = password;
        reveal.showModal();
        setStatus(status, '');
      });
      copy.onclick = runAction(copy, status, async () => {
        if (!navigator.clipboard) throw new Error('Clipboard is unavailable.');
        const { password } = await callWorkspace('login_password', { id: login.id });
        await navigator.clipboard.writeText(password);
        setStatus(status, 'Password copied.');
      });
      remove.onclick = runAction(remove, status, async () => {
        const confirmed = await confirmDashboard('Delete login?', login.origin, 'Delete login');
        if (!confirmed) { setStatus(status, ''); return; }
        await callWorkspace('delete_record', { id: login.id });
        await load();
        restoreRowFocus(list, index, '[data-action=edit]', add);
        setStatus(status, 'Login deleted.');
      });
    });
    setStatus(status, '');
  }

  add.onclick = () => openEditor();
  for (const dialog of [editor, reveal]) dialog.querySelector('[data-close]').onclick = () => dialog.close();
  editor.onclose = () => { form.elements.password.value = ''; };
  reveal.onclose = () => { revealed.textContent = ''; };
  form.onsubmit = runAction(save, formStatus, async () => {
    const id = editing;
    await callWorkspace('save_login', { id, ...Object.fromEntries(new FormData(form)) });
    editor.close();
    await load();
    const edit = [...list.querySelectorAll('[data-action=edit]')].find(button => button.dataset.id === id);
    (edit || add).focus();
    setStatus(status, 'Login saved.');
  });
  load().catch(error => setStatus(status, error.message, true));
}



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

function renderExtensionsDashboard(view, signal) {
  view.innerHTML = `<header class="page-header"><div><h1>Extensions</h1>
    <p>Install Chrome Web Store packages, .crx/.zip archives, or unpacked Manifest V3 folders into a workspace after reviewing permissions.</p></div>
    <a class="btn" href="https://chromewebstore.google.com/">Browse Chrome Web Store</a></header>
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
    <section class="panel" id="ext-review-panel" hidden></section>
    <section class="settings-section" aria-labelledby="installed-extensions-heading">
      <h2 id="installed-extensions-heading">Installed extensions</h2>
      <p class="status" id="ext-list-status" role="status">Loading extensions…</p>
      <div class="script-list" id="installed-extensions"></div>
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
    listEl.innerHTML = exts.map(ext => `<div class="listitem">
      <div class="break"><h3>${escapeText(ext.name)}</h3><small>${escapeText(ext.workspace)} · v${escapeText(ext.version)} · <span>${ext.enabled ? 'Enabled' : 'Disabled'}</span></small></div>
      <div class="script-details break"><span class="script-detail-label">Permissions</span>
        ${escapeText((ext.permissions || []).join(', ') || 'None declared')}</div>
      <div class="script-details break"><span class="script-detail-label">Site access</span>
        ${escapeText((ext.host_permissions || []).join(', ') || (ext.content_scripts || []).flatMap(c => c.matches || []).join(', ') || 'None')}</div>
      <div class="actions">
        ${ext.options_path ? `<a class="btn" href="harbor://extension-asset/${encodeURIComponent(ext.id)}/${encodeURIComponent(ext.version)}/${encodeURIComponent(ext.options_path)}">Options</a>` : ''}
        ${iconToggle('enabled', 'Enable ' + ext.name, ext.enabled, ext.id)}
        ${actionButton('delete', 'Remove extension', ext.id)}
      </div>
    </div>`).join('') || '<p class="empty-state">No extensions installed.</p>';

    listEl.querySelectorAll('.listitem').forEach((row, index) => {
      const ext = exts[index];
      const toggle = row.querySelector('[data-action=enabled]');
      const remove = row.querySelector('[data-action=delete]');
      toggle.onclick = runAction(toggle, listStatus, async () => {
        const enabled = toggle.getAttribute('aria-pressed') !== 'true';
        await callWorkspace('set_extension_enabled', { id: ext.id, enabled });
        toggle.setAttribute('aria-pressed', String(enabled));
        setStatus(listStatus, enabled ? 'Extension enabled.' : 'Extension disabled.');
        await load();
      });
      remove.onclick = runAction(remove, listStatus, async () => {
        if (await confirmDashboard('Remove extension?', ext.name, 'Remove extension')) {
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
    reviewPanel.hidden = false;
    reviewPanel.innerHTML = `<h2>${escapeText(rev.name)} <small class="subtle">v${escapeText(rev.version)}</small></h2>
      ${rev.description ? `<p>${escapeText(rev.description)}</p>` : ''}
      <dl class="review-details">
        <div><dt>Workspace</dt><dd>${escapeText(rev.workspace)}</dd></div>
        <div><dt>Source</dt><dd>${escapeText(rev.source_path)}</dd></div>
        <div><dt>Permissions</dt><dd>${escapeText((rev.permissions || []).join(', ') || 'None')}</dd></div>
        <div><dt>Host access</dt><dd>${escapeText((rev.host_permissions || []).join(', ') || 'Content script rules only')}</dd></div>
        <div><dt>Content scripts</dt><dd>${escapeText(String(rev.content_scripts_count))}</dd></div>
        <div><dt>Package SHA-256</dt><dd><code>${escapeText(rev.digest)}</code></dd></div>
      </dl>
      ${(rev.compatibility_notes && rev.compatibility_notes.length) ? `<div class="status" data-error>${rev.compatibility_notes.map(n => escapeText(n)).join('<br>')}</div>` : ''}
      <div class="row wrap compact">
        <button class="btn primary" type="button" id="ext-install-btn">Install extension</button>
        <button class="btn" type="button" id="ext-decline-btn">Decline</button>
      </div>`;
    reviewPanel.querySelector('#ext-decline-btn').onclick = () => {
      reviewPanel.hidden = true;
      reviewPanel.innerHTML = '';
      setStatus(reviewStatus, 'Extension review declined.');
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
    setStatus(reviewStatus, 'Review permissions and site access before installing.');
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

'use strict';

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

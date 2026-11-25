'use strict';

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

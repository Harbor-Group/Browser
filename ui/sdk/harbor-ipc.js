'use strict';

const workspaceRequests = new Map();
window.__harborWorkspaceReply = ({ id, result }) => {
  const pending = workspaceRequests.get(id);
  if (!pending) return;
  workspaceRequests.delete(id);
  clearTimeout(pending.timer);
  if (result.error) pending.reject(new Error(result.error));
  else pending.resolve(result.data);
};

function callWorkspace(type, payload) {
  return new Promise((resolve, reject) => {
    if (typeof window.__harborPostWorkspace !== 'function') {
      reject(new Error('Harbor’s workspace connection is unavailable. Reload this page and try again.'));
      return;
    }
    const id = crypto.randomUUID();
    const timer = setTimeout(() => {
      workspaceRequests.delete(id);
      reject(new Error('Harbor did not respond. Reload the page and try again.'));
    }, 90000);
    workspaceRequests.set(id, { resolve, reject, timer });
    try {
      window.__harborPostWorkspace(JSON.stringify({ id, command: { type, payload } }));
    } catch (error) {
      clearTimeout(timer);
      workspaceRequests.delete(id);
      reject(error);
    }
  });
}

const escapeText = value => String(value).replace(/[&<>"]/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;'
})[character]);

function currentRoute() {
  if (location.protocol === 'harbor:') return location.hostname || 'home';
  return (location.hash || '#home').slice(1);
}

function setRoute(route) {
  if (location.protocol === 'harbor:') {
    location.href = 'harbor://' + route;
  } else {
    location.hash = route;
  }
}

function setStatus(status, message, error = false) {
  status.textContent = message;
  status.toggleAttribute('data-error', error);
}

function runAction(button, status, action) {
  return async event => {
    event?.preventDefault();
    if (button.disabled) return;
    button.disabled = true;
    setStatus(status, 'Working…');
    try {
      await action();
    } catch (error) {
      setStatus(status, error.message, true);
    } finally {
      button.disabled = false;
    }
  };
}

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

function openSavedAddress(raw) {
  const url = new URL(raw);
  if (['http:', 'https:', 'harbor:'].includes(url.protocol)) location.href = url.href;
}

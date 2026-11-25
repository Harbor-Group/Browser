'use strict';

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

'use strict';

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

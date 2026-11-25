'use strict';

async function renderServer(view, setup, message = '') {
  view.innerHTML = `<header class="page-header"><div><h1>${setup ? 'Set up your Personal workspace' : 'Home Sync Server'}</h1>
    <p>Connect to your Harbor server to sync bookmarks, history, logins, filters, and scripts across your devices.</p></div></header>
    <section class="panel" id="server-controls" aria-label="Server connection"><p role="status">Loading server settings…</p></section>
    <p class="status" id="server-status" role="status"></p>
    ${setup ? '<button class="btn" id="server-skip">Continue without sync</button>' : ''}`;
  const controls = view.querySelector('#server-controls');
  const status = view.querySelector('#server-status');
  const skip = view.querySelector('#server-skip');
  if (skip) skip.onclick = runAction(skip, status, async () => {
    await callWorkspace('skip_setup');
    setRoute('home');
  });
  let server;
  try {
    server = await callWorkspace('state');
  } catch (error) {
    controls.innerHTML = '<p>Harbor couldn’t load your saved server settings.</p><button class="btn" id="server-retry">Try again</button>';
    controls.querySelector('#server-retry').onclick = () => renderServer(view, setup);
    setStatus(status, error.message, true);
    return;
  }
  if (server.unlocked) {
    controls.innerHTML = `<div class="sync-card"><div><h2>Connected</h2><p class="break">${escapeText(server.server_url)}</p></div>
      <div class="row wrap compact">
        <button class="btn primary" id="server-sync">Sync now</button>
        ${setup ? '<button class="btn" id="server-home">Open workspace</button>' : ''}
      </div></div>`;
    const sync = controls.querySelector('#server-sync');
    const home = controls.querySelector('#server-home');
    if (home) home.onclick = () => setRoute('home');
    sync.onclick = runAction(sync, status, async () => {
      const updated = await callWorkspace('sync_now');
      const time = new Date(updated.last_sync * 1000).toLocaleTimeString();
      setStatus(status, 'Synced at ' + time);
    });
    setStatus(status, message || server.error || (server.last_sync
      ? 'Last synced ' + new Date(server.last_sync * 1000).toLocaleString()
      : 'Your workspace will sync automatically while the vault is unlocked.'), Boolean(server.error));
    return;
  }
  controls.innerHTML = `${server.paired ? '' : `
    <form class="settings-form server-form" id="server-address">
      <h2>Server address</h2>
      <label>Hostname or IP address<input class="input" name="server_url" type="text" placeholder="sync.example.com or 192.168.1.20" autocomplete="off" spellcheck="false" required></label>
      <span class="subtle">Use the address of the computer running your Harbor sync server. Include the port if it uses one other than 443.</span>
      <label>TLS certificate fingerprint <span class="subtle">Use the SHA-256 fingerprint from your server administrator if the certificate is privately issued and this device doesn’t trust its CA.</span>
        <input class="input" name="pin" placeholder="Optional for trusted certificates" autocomplete="off" spellcheck="false"></label>
      <div class="row"><button class="btn primary" type="submit">Check connection</button></div>
    </form>`}
    <form class="settings-form server-form" id="server-form" ${server.paired ? '' : 'hidden'}>
    ${server.paired ? `<h2>Unlock vault</h2><p class="break">Resume sync with ${escapeText(server.server_url)}.</p>` : `
      <h2>Pair this device</h2>
      <p id="verified-server" class="break"></p>
      <label>Device name<input class="input" name="device_name" value="Harbor on Windows" maxlength="100" required></label>
      <label>Pairing code<input class="input" name="code" autocomplete="off" spellcheck="false" required></label>
      <span class="subtle">Run <code>sync-server pair-code</code> on your server, or <code>docker compose exec sync-server sync-server pair-code</code> with Docker. The code is single-use and expires after five minutes.</span>`}
    <label>Vault passphrase<input class="input" name="passphrase" type="password" autocomplete="off" required></label>
    ${server.paired ? '' : '<label>Confirm passphrase<input class="input" name="confirmation" type="password" autocomplete="off" required></label>'}
    <span class="subtle">Use the same passphrase on every paired device. The passphrase stays on this device.</span>
    <div class="row"><button class="btn primary" type="submit">${server.paired ? 'Unlock vault' : 'Pair and unlock'}</button>
    ${server.paired ? '' : '<button class="btn" type="button" id="server-back">Back</button>'}</div>
  </form>`;
  const form = controls.querySelector('#server-form');
  const addressForm = controls.querySelector('#server-address');
  let verifiedAddress;
  if (addressForm) {
    addressForm.elements.server_url.value = server.server_url || '';
    addressForm.onsubmit = runAction(addressForm.querySelector('button'), status, async () => {
      const address = Object.fromEntries(new FormData(addressForm));
      const verified = await callWorkspace('check_server', address);
      verifiedAddress = { ...address, server_url: verified.server_url };
      controls.querySelector('#verified-server').textContent = 'Server reachable at ' + verifiedAddress.server_url;
      addressForm.hidden = true;
      form.hidden = false;
      form.elements.code.focus();
      setStatus(status, '');
    });
    controls.querySelector('#server-back').onclick = () => {
      verifiedAddress = undefined;
      form.hidden = true;
      addressForm.hidden = false;
      addressForm.elements.server_url.focus();
      setStatus(status, '');
    };
  }
  if (message || server.error) setStatus(status, message || server.error, true);
  form.onsubmit = runAction(form.querySelector('[type="submit"]'), status, async () => {
    const payload = Object.fromEntries(new FormData(form));
    if (!server.paired) {
      if (!verifiedAddress) throw new Error('Check the server connection first.');
      if (payload.passphrase !== payload.confirmation) throw new Error('The passphrases do not match.');
      Object.assign(payload, verifiedAddress);
      delete payload.confirmation;
      form.elements.confirmation.value = '';
    }
    form.elements.passphrase.value = '';
    try {
      await callWorkspace(server.paired ? 'unlock' : 'pair', payload);
    } catch (error) {
      const refreshed = await callWorkspace('state').catch(() => null);
      if (refreshed && refreshed.paired !== server.paired) {
        await renderServer(view, setup, error.message);
        return;
      }
      throw error;
    }
    if (setup) setRoute('home');
    else await renderServer(view, false, 'Vault unlocked. Automatic sync is on.');
  });
}

'use strict';

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

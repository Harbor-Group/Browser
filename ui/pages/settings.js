'use strict';

function renderSettingsDashboard(view) {
  view.innerHTML = `
    <header class="page-header"><h1>Settings</h1></header>
    <div class="settings-tabs" role="tablist" aria-label="Settings sections">
      <button class="settings-tab" type="button" role="tab" data-settings-tab="general" aria-selected="true">General</button>
      <button class="settings-tab" type="button" role="tab" data-settings-tab="privacy" aria-selected="false">Privacy &amp; DNS</button>
      <button class="settings-tab" type="button" role="tab" data-settings-tab="performance" aria-selected="false">Performance</button>
      <button class="settings-tab" type="button" role="tab" data-settings-tab="security" aria-selected="false">Security</button>
    </div>
    <form class="settings-form" id="home-settings" novalidate>
      <div class="settings-panel" data-settings-panel="general" role="tabpanel">
      <section class="settings-section" aria-labelledby="navigation-settings">
        <h2 id="navigation-settings">Navigation</h2>
        <div class="listitem">
          <div><strong>Show Home button</strong><small id="home-toggle-state">Loading preference…</small></div>
          ${iconToggle('home', 'Show Home button', false)}
        </div>
        <label class="setting-field" id="home-destination" hidden>Home button destination
          <input class="input" name="home_url" value="harbor://home" placeholder="harbor://home or https://example.com" required>
        </label>
      </section>
      <section class="settings-section" aria-labelledby="search-settings">
        <h2 id="search-settings">Search</h2>
        <label class="setting-field">Search engine
          <span class="select-wrap"><select class="input" name="search_engine">
            <option value="duck_duck_go">DuckDuckGo</option><option value="google">Google</option>
            <option value="bing">Bing</option><option value="brave">Brave Search</option><option value="custom">Custom</option>
          </select></span>
        </label>
        <div id="custom-search" hidden>
          <label class="setting-field">Custom search URL
            <input class="input" name="custom_search_url" aria-describedby="search-template-help" placeholder="https://search.example.com/?q={query}" required disabled>
          </label>
          <small class="subtle" id="search-template-help">Put {query} where the search terms go.</small>
        </div>
      </section>
      </div>
      <div class="settings-panel" data-settings-panel="privacy" role="tabpanel" hidden>
      <section class="settings-section" aria-labelledby="dns-settings">
        <h2 id="dns-settings">DNS &amp; Privacy</h2>
        <p class="subtle compact">Encrypt DNS lookups with DNS-over-HTTPS (DoH) or use your system resolver.</p>
        <div class="dns-presets" role="radiogroup" aria-labelledby="dns-settings">
          <label class="dns-card"><input type="radio" name="dns_preset" value="automatic" checked>
            <span class="dns-card-title">Automatic</span><span class="dns-card-sub">OS / network DoH</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="cloudflare">
            <span class="dns-card-title">Cloudflare</span><span class="dns-card-sub">1.1.1.1 · Fast &amp; private</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="quad9">
            <span class="dns-card-title">Quad9</span><span class="dns-card-sub">9.9.9.9 · Malware block</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="adguard">
            <span class="dns-card-title">AdGuard DNS</span><span class="dns-card-sub">Blocks ads &amp; trackers</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="mullvad">
            <span class="dns-card-title">Mullvad DNS</span><span class="dns-card-sub">No-logging · Privacy</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="google">
            <span class="dns-card-title">Google</span><span class="dns-card-sub">8.8.8.8 · Public DNS</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="off">
            <span class="dns-card-title">Off</span><span class="dns-card-sub">Standard system DNS</span></label>
          <label class="dns-card"><input type="radio" name="dns_preset" value="custom">
            <span class="dns-card-title">Custom</span><span class="dns-card-sub">Your DoH endpoint</span></label>
        </div>
        <div class="dns-endpoint" id="dns-endpoint-hint"></div>
        <div id="custom-dns" hidden>
          <label class="setting-field">Custom DNS-over-HTTPS URL
            <input class="input" name="custom_dns_url" type="url" aria-describedby="dns-template-help" placeholder="https://dns.example.com/dns-query" required disabled>
          </label>
          <small class="subtle" id="dns-template-help">Enter an HTTPS DNS-over-HTTPS endpoint.</small>
        </div>
      </section>
      </div>
      <div class="settings-panel" data-settings-panel="performance" role="tabpanel" hidden>
      <section class="settings-section" aria-labelledby="background-tab-settings">
        <h2 id="background-tab-settings">Background Tabs &amp; Performance</h2>
        <p class="subtle compact">Keep background tabs running without GPU rendering so music, audio, and web apps stay alive.</p>
        <div class="listitem">
          <div><strong>Keep background tabs alive</strong><small id="bg-tabs-state">Loading preference…</small></div>
          ${iconToggle('bg-tabs', 'Keep background tabs alive', true)}
        </div>
        <div id="bg-tabs-options">
          <label class="setting-field">Maximum live background tabs
            <span class="select-wrap"><select class="input" name="background_tab_limit">
              <option value="3">3 background tabs</option>
              <option value="5">5 background tabs</option>
              <option value="8" selected>8 background tabs (Recommended)</option>
              <option value="12">12 background tabs</option>
              <option value="0">Unlimited</option>
            </select></span>
          </label>
          <label class="setting-field">Put inactive background tabs to sleep after
            <span class="select-wrap"><select class="input" name="sleep_tabs_after_minutes">
              <option value="5">5 minutes</option>
              <option value="15">15 minutes</option>
              <option value="30" selected>30 minutes (Recommended)</option>
              <option value="60">1 hour</option>
              <option value="0">Never</option>
            </select></span>
          </label>
          <div class="listitem">
            <div><strong>Never sleep tabs playing audio</strong><small id="bg-audio-state">Keeps music players like Spotify active in the background</small></div>
            ${iconToggle('bg-audio', 'Never sleep tabs playing audio', true)}
          </div>
        </div>
      </section>
      </div>
      <div class="row" id="settings-save-row"><button class="btn primary" type="submit" disabled>Save settings</button></div>
      <p class="status" id="home-status" role="status"></p>
    </form>
    <div class="settings-panel" data-settings-panel="security" role="tabpanel" hidden>
    <section class="section settings-section" aria-labelledby="security-settings">
      <h2 id="security-settings">Security</h2>
      <div class="listitem">
        <div><strong>Local record encryption</strong><small>Authenticated encryption with a key in Windows Credential Manager</small></div>
      </div>
      <div class="listitem">
        <div><strong>Sync connection</strong><small>TLS 1.3 with public certificate validation or a configured fingerprint</small></div>
      </div>
    </section>
    </div>`;

  const tabButtons = view.querySelectorAll('[data-settings-tab]');
  const tabPanels = view.querySelectorAll('[data-settings-panel]');
  const saveRow = view.querySelector('#settings-save-row');
  const selectSettingsTab = name => {
    tabButtons.forEach(btn => btn.setAttribute('aria-selected', String(btn.dataset.settingsTab === name)));
    tabPanels.forEach(panel => { panel.hidden = panel.dataset.settingsPanel !== name; });
    if (saveRow) saveRow.hidden = name === 'security';
  };
  tabButtons.forEach(btn => { btn.onclick = () => selectSettingsTab(btn.dataset.settingsTab); });

  const form = view.querySelector('#home-settings');
  const status = form.querySelector('#home-status');
  const save = form.querySelector('[type=submit]');
  const toggle = form.querySelector('[data-action=home]');
  const toggleState = form.querySelector('#home-toggle-state');
  const destination = form.querySelector('#home-destination');
  const bgToggle = form.querySelector('[data-action=bg-tabs]');
  const bgToggleState = form.querySelector('#bg-tabs-state');
  const bgOptions = form.querySelector('#bg-tabs-options');
  const bgAudioToggle = form.querySelector('[data-action=bg-audio]');
  const custom = form.querySelector('#custom-search');
  const customDns = form.querySelector('#custom-dns');
  const dnsHint = form.querySelector('#dns-endpoint-hint');
  const dnsEndpoints = {
    automatic: 'Active resolver: Automatic (uses system/network DoH when supported)',
    cloudflare: 'Active endpoint: https://cloudflare-dns.com/dns-query',
    quad9: 'Active endpoint: https://dns.quad9.net/dns-query',
    adguard: 'Active endpoint: https://dns.adguard-dns.com/dns-query',
    mullvad: 'Active endpoint: https://doh.mullvad.net/dns-query',
    google: 'Active endpoint: https://dns.google/dns-query',
    off: 'Active resolver: Standard unencrypted system DNS',
    custom: ''
  };
  let savedHomeUrl;
  const showHome = enabled => {
    toggle.setAttribute('aria-pressed', String(enabled));
    toggleState.textContent = enabled ? 'On · Appears next to Back and Forward' : 'Off · Hidden from the toolbar';
    destination.hidden = !enabled;
    form.elements.home_url.disabled = !enabled;
  };
  toggle.disabled = true;
  toggle.setAttribute('aria-describedby', 'home-toggle-state');
  toggle.onclick = () => showHome(toggle.getAttribute('aria-pressed') !== 'true');
  const showBgTabs = enabled => {
    bgToggle.setAttribute('aria-pressed', String(enabled));
    bgToggleState.textContent = enabled ? 'On · Background tabs stay live without rendering' : 'Off · Inactive tabs unload immediately';
    bgOptions.hidden = !enabled;
    form.elements.background_tab_limit.disabled = !enabled;
    form.elements.sleep_tabs_after_minutes.disabled = !enabled;
    bgAudioToggle.disabled = !enabled || save.disabled;
  };
  const showBgAudio = enabled => {
    bgAudioToggle.setAttribute('aria-pressed', String(enabled));
  };
  bgToggle.disabled = true;
  bgAudioToggle.disabled = true;
  bgToggle.onclick = () => showBgTabs(bgToggle.getAttribute('aria-pressed') !== 'true');
  bgAudioToggle.onclick = () => showBgAudio(bgAudioToggle.getAttribute('aria-pressed') !== 'true');
  const showCustom = () => {
    custom.hidden = form.elements.search_engine.value !== 'custom';
    form.elements.custom_search_url.disabled = custom.hidden;
  };
  const showDns = () => {
    const preset = form.elements.dns_preset.value || 'automatic';
    customDns.hidden = preset !== 'custom';
    form.elements.custom_dns_url.disabled = customDns.hidden;
    dnsHint.textContent = dnsEndpoints[preset] || '';
    dnsHint.hidden = !dnsHint.textContent;
  };
  form.elements.search_engine.onchange = showCustom;
  form.querySelectorAll('input[name=dns_preset]').forEach(radio => { radio.onchange = showDns; });
  const displaySettings = settings => {
    showHome(settings.show_home_button);
    savedHomeUrl = settings.home_url;
    form.elements.home_url.value = settings.home_url;
    form.elements.search_engine.value = settings.search_engine;
    form.elements.custom_search_url.value = settings.custom_search_url;
    form.elements.dns_preset.value = settings.dns_preset || 'automatic';
    form.elements.custom_dns_url.value = settings.custom_dns_url || '';
    showBgTabs(settings.background_tabs_enabled !== false);
    form.elements.background_tab_limit.value = String(settings.background_tab_limit ?? 8);
    form.elements.sleep_tabs_after_minutes.value = String(settings.sleep_tabs_after_minutes ?? 30);
    showBgAudio(settings.sleep_preserve_audio !== false);
    showCustom();
    showDns();
  };
  form.onsubmit = runAction(save, status, async () => {
    const payload = {
      show_home_button: toggle.getAttribute('aria-pressed') === 'true',
      home_url: destination.hidden ? savedHomeUrl : form.elements.home_url.value,
      search_engine: form.elements.search_engine.value,
      custom_search_url: form.elements.custom_search_url.value,
      dns_preset: form.elements.dns_preset.value || 'automatic',
      custom_dns_url: form.elements.custom_dns_url.value,
      background_tabs_enabled: bgToggle.getAttribute('aria-pressed') === 'true',
      background_tab_limit: Number(form.elements.background_tab_limit.value),
      sleep_tabs_after_minutes: Number(form.elements.sleep_tabs_after_minutes.value),
      sleep_preserve_audio: bgAudioToggle.getAttribute('aria-pressed') === 'true'
    };
    toggle.disabled = bgToggle.disabled = bgAudioToggle.disabled = form.elements.search_engine.disabled = true;
    form.elements.background_tab_limit.disabled = form.elements.sleep_tabs_after_minutes.disabled = true;
    form.querySelectorAll('input[name=dns_preset]').forEach(r => { r.disabled = true; });
    form.elements.home_url.readOnly = form.elements.custom_search_url.readOnly = form.elements.custom_dns_url.readOnly = true;
    try {
      displaySettings(await callWorkspace('save_home_settings', payload));
      setStatus(status, 'Settings saved.');
    } finally {
      toggle.disabled = bgToggle.disabled = form.elements.search_engine.disabled = false;
      showBgTabs(bgToggle.getAttribute('aria-pressed') === 'true');
      form.querySelectorAll('input[name=dns_preset]').forEach(r => { r.disabled = false; });
      form.elements.home_url.readOnly = form.elements.custom_search_url.readOnly = form.elements.custom_dns_url.readOnly = false;
    }
  });
  callWorkspace('state').then(settings => {
    save.disabled = toggle.disabled = bgToggle.disabled = false;
    displaySettings(settings);
  }).catch(error => setStatus(status, error.message, true));
}

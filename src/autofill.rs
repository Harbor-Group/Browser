/// Lightweight renderer content script injected into HTTPS pages.
/// Detects password forms and reports form submission events to the Harbor Rust IPC bridge.
pub const AUTOFILL_CONTENT_SCRIPT: &str = r#"
(function() {
  'use strict';
  if (window.__harborAutofillInstalled) return;
  window.__harborAutofillInstalled = true;

  function findUsernameInput(form, pwdInput) {
    const inputs = Array.from(form.querySelectorAll('input'));
    const idx = inputs.indexOf(pwdInput);
    for (let i = idx - 1; i >= 0; i--) {
      const t = (inputs[i].type || 'text').toLowerCase();
      if (t === 'email' || t === 'text' || t === 'tel') return inputs[i];
    }
    return null;
  }

  document.addEventListener('submit', function(ev) {
    const form = ev.target;
    if (!(form instanceof HTMLFormElement)) return;
    const pwd = form.querySelector('input[type="password"]');
    if (!pwd || !pwd.value) return;
    const user = findUsernameInput(form, pwd);
    if (window.__harborIPC && typeof window.__harborIPC.offerSaveLogin === 'function') {
      window.__harborIPC.offerSaveLogin({
        origin: window.location.origin,
        username: user ? user.value : '',
        password: pwd.value
      });
    }
  }, true);
})();
"#;

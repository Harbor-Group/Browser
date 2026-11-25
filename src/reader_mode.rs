/// Lightweight zero-dependency Reader Mode toggle script injected into active tabs.
/// Extracts the primary article container and renders a clean, distraction-free serif view.
pub const READER_MODE_TOGGLE_SCRIPT: &str = r#"
(function() {
  'use strict';
  const OVERLAY_ID = '__harbor_reader_overlay';
  const existing = document.getElementById(OVERLAY_ID);
  if (existing) {
    existing.remove();
    document.documentElement.style.overflow = '';
    return;
  }

  const candidate =
    document.querySelector('article') ||
    document.querySelector('main') ||
    document.querySelector('[role="main"]') ||
    document.body;

  const title =
    (document.querySelector('h1') && document.querySelector('h1').innerText.trim()) ||
    document.title ||
    location.hostname;

  const blocks = Array.from(candidate.querySelectorAll('p, h2, h3, blockquote, pre, ul, ol'))
    .filter(el => el.innerText && el.innerText.trim().length > 20)
    .map(el => el.outerHTML)
    .join('\n');

  const overlay = document.createElement('div');
  overlay.id = OVERLAY_ID;
  overlay.style.cssText = [
    'position:fixed;inset:0;z-index:2147483647;overflow-y:auto;',
    'background:#181b20;color:#e5e7eb;padding:48px 24px 96px;',
    'font-family:Georgia,Cambria,"Times New Roman",serif;line-height:1.85;font-size:18px;'
  ].join('');

  overlay.innerHTML = `
    <div style="max-width:680px;margin:0 auto;">
      <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:24px;font-family:Inter,system-ui,sans-serif;font-size:12px;color:#94a3b8;">
        <span>HARBOR READER MODE · ${location.hostname}</span>
        <button id="__harbor_reader_exit" style="background:#27313f;color:#e5e7eb;border:1px solid #334155;border-radius:6px;padding:5px 10px;cursor:pointer;">Exit Reader</button>
      </div>
      <h1 style="font-size:32px;line-height:1.25;color:#f8fafc;margin:0 0 24px;"></h1>
      <div id="__harbor_reader_body" style="color:#d1d5db;"></div>
    </div>
  `;

  overlay.querySelector('h1').textContent = title;
  overlay.querySelector('#__harbor_reader_body').innerHTML =
    blocks || '<p>No readable article content detected on this page.</p>';

  overlay.querySelector('#__harbor_reader_exit').onclick = () => {
    overlay.remove();
    document.documentElement.style.overflow = '';
  };

  document.documentElement.style.overflow = 'hidden';
  document.body.appendChild(overlay);
})();
"#;

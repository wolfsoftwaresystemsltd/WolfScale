// WolfScale Website Scripts

// Apply saved theme immediately to prevent flash
(function () {
    const savedTheme = localStorage.getItem('wolfscale-theme') || 'dark';
    document.documentElement.setAttribute('data-theme', savedTheme);
})();

// ---- Site-wide Support Banner ----
// One-liner banner on every page except support.html.
// Dismissed state persists for 1 day via localStorage.
(function () {
    const DISMISS_KEY = 'wolfstack-support-banner-v2';
    const DISMISS_MS = 24 * 60 * 60 * 1000; // 1 day
    const isSupport = window.location.pathname.endsWith('support.html');
    if (isSupport) return;

    const raw = localStorage.getItem(DISMISS_KEY);
    if (raw && (Date.now() - parseInt(raw, 10)) < DISMISS_MS) return;

    const banner = document.createElement('div');
    banner.id = 'support-banner';
    // Inline styles so nothing can accidentally hide/override it
    banner.style.cssText = [
        'background:#b91c1c',
        'color:#ffffff',
        'font-family:Inter,-apple-system,sans-serif',
        'font-size:0.78rem',
        'font-weight:500',
        'line-height:1',
        'padding:8px 40px 8px 16px',
        'text-align:center',
        'position:relative',
        'z-index:9999',
        'white-space:nowrap',
        'overflow:hidden',
        'text-overflow:ellipsis',
        'border-bottom:1px solid rgba(255,255,255,0.2)'
    ].join(';');
    banner.innerHTML =
        '🐺 <strong>WolfStack is free &amp; open-source</strong> — help keep it alive &nbsp;' +
        '<a href="support.html" style="color:#fde68a;font-weight:700;text-decoration:underline;">❤️ Become a Patron</a>' +
        '<button id="banner-close-btn" aria-label="Dismiss" style="position:absolute;right:10px;top:50%;transform:translateY(-50%);background:none;border:none;color:rgba(255,255,255,0.8);cursor:pointer;font-size:1rem;line-height:1;padding:2px 4px;">✕</button>';

    document.body.insertBefore(banner, document.body.firstChild);

    document.getElementById('banner-close-btn').addEventListener('click', function () {
        banner.remove();
        localStorage.setItem(DISMISS_KEY, String(Date.now()));
    });
})();

document.addEventListener('DOMContentLoaded', function () {
    // Theme toggle functionality
    const themeToggle = document.getElementById('theme-toggle');

    function setTheme(theme) {
        document.documentElement.setAttribute('data-theme', theme);
        localStorage.setItem('wolfscale-theme', theme);
    }

    if (themeToggle) {
        themeToggle.addEventListener('click', function () {
            const currentTheme = document.documentElement.getAttribute('data-theme') || 'dark';
            const newTheme = currentTheme === 'dark' ? 'light' : 'dark';
            setTheme(newTheme);
        });
    }
    // Mobile menu toggle for landing page
    const mobileMenuBtn = document.getElementById('mobile-menu-btn');
    const navLinks = document.getElementById('nav-links');

    if (mobileMenuBtn && navLinks) {
        mobileMenuBtn.addEventListener('click', function () {
            mobileMenuBtn.classList.toggle('active');
            navLinks.classList.toggle('active');
        });
    }

    // Sidebar toggle for wiki pages
    const sidebar = document.getElementById('sidebar');
    const sidebarOverlay = document.getElementById('sidebar-overlay');

    if (mobileMenuBtn && sidebar) {
        mobileMenuBtn.addEventListener('click', function () {
            mobileMenuBtn.classList.toggle('active');
            sidebar.classList.toggle('active');
            if (sidebarOverlay) {
                sidebarOverlay.classList.toggle('active');
            }
        });
    }

    // Close sidebar when clicking overlay
    if (sidebarOverlay) {
        sidebarOverlay.addEventListener('click', function () {
            sidebar.classList.remove('active');
            sidebarOverlay.classList.remove('active');
            if (mobileMenuBtn) {
                mobileMenuBtn.classList.remove('active');
            }
        });
    }

    // Copy button functionality is handled by the global copyCode function below

    // Smooth scroll for anchor links
    document.querySelectorAll('a[href^="#"]').forEach(anchor => {
        anchor.addEventListener('click', function (e) {
            const target = document.querySelector(this.getAttribute('href'));
            if (target) {
                e.preventDefault();
                target.scrollIntoView({ behavior: 'smooth', block: 'start' });
            }
        });
    });
});

// Global copy function called by inline onclick="copyCode(this)" handlers
function copyCode(btn) {
    const codeBlock = btn.closest('.code-block');
    if (!codeBlock) return;
    const code = codeBlock.querySelector('code');
    if (!code) return;
    const text = code.textContent;

    if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(() => {
            btn.textContent = 'Copied!';
            setTimeout(() => { btn.textContent = 'Copy'; }, 2000);
        }).catch(() => {
            fallbackCopy(text, btn);
        });
    } else {
        fallbackCopy(text, btn);
    }
}

function fallbackCopy(text, btn) {
    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.style.position = 'fixed';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    try {
        document.execCommand('copy');
        btn.textContent = 'Copied!';
        setTimeout(() => { btn.textContent = 'Copy'; }, 2000);
    } catch (e) {
        btn.textContent = 'Failed';
        setTimeout(() => { btn.textContent = 'Copy'; }, 2000);
    }
    document.body.removeChild(textarea);
}

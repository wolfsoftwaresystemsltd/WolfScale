// WolfStack website scripts v20

// Theme: apply saved preference before paint
(function () {
    var t = localStorage.getItem('wolfscale-theme') || 'light';
    document.documentElement.setAttribute('data-theme', t);
})();


document.addEventListener('DOMContentLoaded', function () {
    // Theme toggle
    var toggle = document.getElementById('theme-toggle');
    if (toggle) {
        toggle.addEventListener('click', function () {
            var cur = document.documentElement.getAttribute('data-theme') || 'light';
            var next = cur === 'dark' ? 'light' : 'dark';
            document.documentElement.setAttribute('data-theme', next);
            localStorage.setItem('wolfscale-theme', next);
        });
    }

    // Mobile menu (landing page)
    var menuBtn = document.getElementById('mobile-menu-btn');
    var navLinks = document.getElementById('nav-links');
    if (menuBtn && navLinks) {
        menuBtn.addEventListener('click', function () {
            menuBtn.classList.toggle('active');
            navLinks.classList.toggle('active');
        });
    }

    // Sidebar toggle (doc pages)
    var sidebar = document.getElementById('sidebar');
    var overlay = document.getElementById('sidebar-overlay');

    if (menuBtn && sidebar) {
        menuBtn.addEventListener('click', function () {
            menuBtn.classList.toggle('active');
            sidebar.classList.toggle('active');
            if (overlay) overlay.classList.toggle('active');
        });
    }

    if (overlay) {
        overlay.addEventListener('click', function () {
            sidebar.classList.remove('active');
            overlay.classList.remove('active');
            if (menuBtn) menuBtn.classList.remove('active');
        });
    }

    // Smooth scroll for anchor links
    document.querySelectorAll('a[href^="#"]').forEach(function (a) {
        a.addEventListener('click', function (e) {
            var target = document.querySelector(this.getAttribute('href'));
            if (target) {
                e.preventDefault();
                target.scrollIntoView({ behavior: 'smooth', block: 'start' });
            }
        });
    });
});

// Copy code button handler (called via onclick)
function copyCode(btn) {
    var block = btn.closest('.code-block');
    if (!block) return;
    var code = block.querySelector('code');
    if (!code) return;

    var text = code.textContent;

    if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(function () {
            btn.textContent = 'Copied!';
            setTimeout(function () { btn.textContent = 'Copy'; }, 2000);
        }).catch(function () {
            fallbackCopy(text, btn);
        });
    } else {
        fallbackCopy(text, btn);
    }
}

function fallbackCopy(text, btn) {
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try {
        document.execCommand('copy');
        btn.textContent = 'Copied!';
    } catch (e) {
        btn.textContent = 'Failed';
    }
    setTimeout(function () { btn.textContent = 'Copy'; }, 2000);
    document.body.removeChild(ta);
}

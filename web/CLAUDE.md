# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Static documentation and marketing website for the Wolf product suite, hosted at **wolfscale.org**. No build system, no framework, no package manager — plain HTML, CSS, and vanilla JavaScript served as static files.

## Serving Locally

Open any HTML file directly in a browser, or use any static file server:

```bash
python3 -m http.server 8080    # then visit http://localhost:8080
```

## Site Structure

- **`index.html`** — Landing/marketing page (~77KB, contains significant inline `<style>` and animation CSS)
- **`styles.css`** — Shared stylesheet. Uses CSS custom properties for theming (dark default, light via `[data-theme="light"]`). Red accent palette (`--accent-primary: #dc2626`).
- **`script.js`** — Shared JS: theme toggle (persisted to `localStorage`), mobile sidebar, support banner injection, copy-to-clipboard for code blocks.
- **`_sidebar.html`** — Canonical sidebar navigation source, but **not dynamically included** — the sidebar markup is copy-pasted into every doc page. When updating navigation, every HTML file must be updated.
- **`sitemap.xml`** / **`robots.txt`** — SEO files for wolfscale.org.

### Doc Pages (~40 HTML files)

Each page follows an identical template:
1. Google Analytics snippet (GA4: `G-KK9NEE1S54`)
2. `<meta>` tags (description, OG, Twitter)
3. Google Fonts (Inter + JetBrains Mono)
4. `<link rel="stylesheet" href="styles.css?v=10">`
5. `<div class="wiki-layout">` containing:
   - Full sidebar copy (same as `_sidebar.html` with `class="active"` on current page link)
   - `<main class="wiki-content">` with the page content
6. `<script src="script.js?v=7"></script>` before `</body>`

### Product Sections (in sidebar order)

| Section | Pages | Description |
|---|---|---|
| WolfStack | `wolfstack*.html`, `wolfrun.html`, `proxmox.html`, `app-store.html` | Server management dashboard docs |
| WolfScale | `quickstart.html`, `features.html`, `architecture.html`, `how-it-works.html`, `binlog.html`, `load-balancer.html`, `configuration.html`, `performance.html`, `cli.html`, `troubleshooting.html` | Database replication & load balancer |
| WolfDisk | `wolfdisk.html` | Disk replication & sharing |
| WolfNet | `wolfnet.html`, `wolfnet-global.html`, `wolfnet-vpn.html` | Private networking / VPN |
| WolfProxy | `wolfproxy.html` | NGINX-compatible reverse proxy |
| WolfServe | `wolfserve.html` | Apache2-compatible web server |
| Company | `about.html`, `roadmap.html`, `contact.html`, `glossary.html`, `licensing.html`, `enterprise.html`, `support.html` | Corporate/info pages |

## Key Patterns

- **Theme system**: Dark/light toggle via `data-theme` attribute on `<html>`. All colors reference CSS variables. Theme preference stored in `localStorage` key `wolfscale-theme`.
- **Code blocks**: Use `<div class="code-block">` with a `<button onclick="copyCode(this)">Copy</button>` and `<code>` element inside.
- **Support banner**: Injected at top of every page by `script.js` (except `support.html`). Adjusts sidebar top offset.
- **No templating**: Sidebar and boilerplate are duplicated across all files. The `_sidebar.html` file exists as a reference but isn't loaded dynamically.
- **Cache busting**: Query params on CSS/JS links (`styles.css?v=10`, `script.js?v=7`) — bump manually when changing shared assets.

## Assets

- **`images/`** — Product screenshots and logos
- **`favicon.png`** — Site favicon (536KB — oversized)
- **`slide1.png`**, **`slide2.png`** — Landing page hero images
- **Logo**: `images/wolfstack-logo.png` (400x110, 23KB)

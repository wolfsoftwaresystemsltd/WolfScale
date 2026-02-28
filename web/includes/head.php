<!DOCTYPE html>
<html lang="en">
<head>
    <script async src="https://www.googletagmanager.com/gtag/js?id=G-KK9NEE1S54"></script>
    <script>window.dataLayer=window.dataLayer||[];function gtag(){dataLayer.push(arguments)}gtag('js',new Date());gtag('config','G-KK9NEE1S54');</script>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="description" content="<?php echo htmlspecialchars($page_desc ?? 'WolfStack — The Universal Server Management Platform'); ?>">
    <meta name="keywords" content="<?php echo htmlspecialchars($page_keywords ?? 'server management, WolfStack, dashboard, Docker, LXC, monitoring, clustering, WolfScale, WolfDisk, WolfNet'); ?>">
    <meta name="author" content="Wolf Software Systems Ltd">
<?php if (!empty($page_canonical)): ?>
    <link rel="canonical" href="<?php echo $page_canonical; ?>">
<?php endif; ?>

    <!-- Open Graph -->
    <meta property="og:type" content="website">
<?php if (!empty($page_canonical)): ?>
    <meta property="og:url" content="<?php echo $page_canonical; ?>">
<?php endif; ?>
    <meta property="og:title" content="<?php echo htmlspecialchars($page_title ?? 'WolfStack'); ?>">
    <meta property="og:description" content="<?php echo htmlspecialchars($page_desc ?? 'WolfStack — The Universal Server Management Platform'); ?>">
    <meta property="og:image" content="images/wolfstack-logo.png">

    <!-- Twitter -->
    <meta property="twitter:card" content="summary_large_image">
<?php if (!empty($page_canonical)): ?>
    <meta property="twitter:url" content="<?php echo $page_canonical; ?>">
<?php endif; ?>
    <meta property="twitter:title" content="<?php echo htmlspecialchars($page_title ?? 'WolfStack'); ?>">
    <meta property="twitter:description" content="<?php echo htmlspecialchars($page_desc ?? 'WolfStack — The Universal Server Management Platform'); ?>">

    <title><?php echo htmlspecialchars($page_title ?? 'WolfStack'); ?></title>
    <link rel="icon" type="image/png" href="favicon.png">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
    <link rel="stylesheet" href="styles.css?v=24">
    <script>
        (function(){var t=localStorage.getItem('wolfscale-theme')||'light';document.documentElement.setAttribute('data-theme',t)})();
    </script>
<?php if (!empty($page_css)): ?>
    <style><?php echo $page_css; ?></style>
<?php endif; ?>
</head>

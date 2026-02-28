<?php
session_start();
require_once 'includes/mail-send.php';

$page_title = 'Contact Sales — WolfStack Enterprise';
$page_desc = 'Get in touch with our sales team about WolfStack Enterprise licensing.';
$active = 'enterprise.php';

// Anti-spam: rate limit (max 3 per 10 min)
if (!isset($_SESSION['contact_submissions'])) $_SESSION['contact_submissions'] = [];

$errors = [];
$success = false;
$form = [
    'name'    => '',
    'email'   => '',
    'company' => '',
    'phone'   => '',
    'plan'    => $_GET['plan'] ?? '',
    'message' => '',
];

if ($_SERVER['REQUEST_METHOD'] === 'POST') {
    // CSRF check
    if (!isset($_POST['_token']) || !isset($_SESSION['csrf_token']) || !hash_equals($_SESSION['csrf_token'], $_POST['_token'])) {
        $errors[] = 'Invalid form submission. Please try again.';
    }

    // Honeypot
    if (!empty($_POST['website'] ?? '')) {
        // Bot detected — show fake success, do nothing
        $success = true;
        goto render;
    }

    // Timing check (must take > 3 seconds)
    $ts = (int)($_POST['_ts'] ?? 0);
    if ($ts > 0 && (time() - $ts) < 3) {
        $errors[] = 'Form submitted too quickly. Please wait a moment and try again.';
    }

    // Rate limit
    $now = time();
    $_SESSION['contact_submissions'] = array_filter(
        $_SESSION['contact_submissions'],
        fn($t) => ($now - $t) < 600
    );
    if (count($_SESSION['contact_submissions']) >= 3) {
        $errors[] = 'Too many submissions. Please try again in a few minutes.';
    }

    // Collect & validate
    $form['name']    = trim($_POST['name'] ?? '');
    $form['email']   = trim($_POST['email'] ?? '');
    $form['company'] = trim($_POST['company'] ?? '');
    $form['phone']   = trim($_POST['phone'] ?? '');
    $form['plan']    = trim($_POST['plan'] ?? '');
    $form['message'] = trim($_POST['message'] ?? '');

    if ($form['name'] === '')    $errors[] = 'Name is required.';
    if ($form['email'] === '')   $errors[] = 'Email is required.';
    elseif (!filter_var($form['email'], FILTER_VALIDATE_EMAIL)) $errors[] = 'Please enter a valid email address.';
    if ($form['company'] === '') $errors[] = 'Company name is required.';
    if ($form['message'] === '') $errors[] = 'Message is required.';
    elseif (strlen($form['message']) < 10)   $errors[] = 'Message must be at least 10 characters.';
    elseif (strlen($form['message']) > 5000) $errors[] = 'Message must be under 5000 characters.';

    $valid_plans = ['Basic', 'Standard', 'Premium', 'Not Sure', ''];
    if (!in_array($form['plan'], $valid_plans)) $form['plan'] = '';

    if (empty($errors)) {
        $plan_label = $form['plan'] ?: 'Not specified';
        $body  = "New enterprise enquiry from wolfscale.org\r\n";
        $body .= "========================================\r\n\r\n";
        $body .= "Name:    {$form['name']}\r\n";
        $body .= "Email:   {$form['email']}\r\n";
        $body .= "Company: {$form['company']}\r\n";
        $body .= "Phone:   " . ($form['phone'] ?: 'Not provided') . "\r\n";
        $body .= "Plan:    $plan_label\r\n\r\n";
        $body .= "Message:\r\n";
        $body .= "--------\r\n";
        $body .= $form['message'] . "\r\n";

        $recipients = ['paul@wolf.uk.com', 'ian@wolf.uk.com', 'ben@wolf.uk.com'];
        $result = true;
        foreach ($recipients as $to) {
            $r = smtp_send($to, 'WolfStack Enterprise Enquiry', $body, $form['email']);
            if ($r !== true) $result = $r;
        }

        if ($result === true) {
            $success = true;
            $_SESSION['contact_submissions'][] = time();
            // Clear form on success
            $form = ['name'=>'','email'=>'','company'=>'','phone'=>'','plan'=>'','message'=>''];
        } else {
            $errors[] = 'Unable to send your message right now. Please email sales@wolf.uk.com directly.';
        }
    }
}

render:
// Generate CSRF token
$_SESSION['csrf_token'] = bin2hex(random_bytes(32));

include 'includes/head.php';
?>
<body>
<div class="wiki-layout">
    <?php include 'includes/sidebar.php'; ?>
    <main class="wiki-content">

        <div class="content-section">
            <h2>Contact Sales</h2>
            <p style="color:var(--text-secondary);margin-bottom:1.5rem;">Fill in the form below and our sales team will get back to you within one business day. For general questions, visit our <a href="contact.php">contact page</a> or join our <a href="https://discord.gg/q9qMjHjUQY" target="_blank">Discord</a>.</p>

            <?php if ($success): ?>
                <div style="background:rgba(34,197,94,0.1);border:1px solid rgba(34,197,94,0.3);border-radius:10px;padding:1.5rem 2rem;margin-bottom:2rem;">
                    <h3 style="color:#22c55e;margin-bottom:0.5rem;">Message sent</h3>
                    <p style="color:var(--text-secondary);margin:0;">Thank you for your enquiry. Our sales team will be in touch within one business day.</p>
                    <p style="margin-top:1rem;"><a href="enterprise.php" style="color:var(--accent-primary);font-weight:600;">&larr; Back to Enterprise Plans</a></p>
                </div>
            <?php else: ?>

                <?php if (!empty($errors)): ?>
                    <div style="background:rgba(220,38,38,0.1);border:1px solid rgba(220,38,38,0.3);border-radius:10px;padding:1rem 1.5rem;margin-bottom:1.5rem;">
                        <?php foreach ($errors as $e): ?>
                            <p style="color:#ef4444;margin:0.25rem 0;font-size:0.9rem;"><?php echo htmlspecialchars($e); ?></p>
                        <?php endforeach; ?>
                    </div>
                <?php endif; ?>

                <form method="post" action="enterprise-contact.php" style="max-width:600px;">
                    <input type="hidden" name="_token" value="<?php echo $_SESSION['csrf_token']; ?>">
                    <input type="hidden" name="_ts" value="<?php echo time(); ?>">
                    <!-- Honeypot -->
                    <div style="position:absolute;left:-9999px;" aria-hidden="true">
                        <input type="text" name="website" tabindex="-1" autocomplete="off">
                    </div>

                    <div style="margin-bottom:1rem;">
                        <label for="name" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Name <span style="color:#ef4444;">*</span></label>
                        <input type="text" id="name" name="name" required value="<?php echo htmlspecialchars($form['name']); ?>"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;">
                    </div>

                    <div style="margin-bottom:1rem;">
                        <label for="email" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Email <span style="color:#ef4444;">*</span></label>
                        <input type="email" id="email" name="email" required value="<?php echo htmlspecialchars($form['email']); ?>"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;">
                    </div>

                    <div style="margin-bottom:1rem;">
                        <label for="company" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Company <span style="color:#ef4444;">*</span></label>
                        <input type="text" id="company" name="company" required value="<?php echo htmlspecialchars($form['company']); ?>"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;">
                    </div>

                    <div style="margin-bottom:1rem;">
                        <label for="phone" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Phone <span style="color:var(--text-muted);font-weight:400;">(optional)</span></label>
                        <input type="tel" id="phone" name="phone" value="<?php echo htmlspecialchars($form['phone']); ?>"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;">
                    </div>

                    <div style="margin-bottom:1rem;">
                        <label for="plan" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Plan Interest</label>
                        <select id="plan" name="plan"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;">
                            <option value="" <?php echo $form['plan'] === '' ? 'selected' : ''; ?>>Not Sure</option>
                            <option value="Basic" <?php echo $form['plan'] === 'Basic' ? 'selected' : ''; ?>>Basic (£95 / year & socket)</option>
                            <option value="Standard" <?php echo $form['plan'] === 'Standard' ? 'selected' : ''; ?>>Standard (£450 / year & socket)</option>
                            <option value="Premium" <?php echo $form['plan'] === 'Premium' ? 'selected' : ''; ?>>Premium (£900 / year & socket)</option>
                        </select>
                    </div>

                    <div style="margin-bottom:1.5rem;">
                        <label for="message" style="display:block;font-weight:600;margin-bottom:0.35rem;font-size:0.9rem;color:var(--text-primary);">Message <span style="color:#ef4444;">*</span></label>
                        <textarea id="message" name="message" required rows="6" minlength="10" maxlength="5000"
                            style="width:100%;padding:10px 14px;border-radius:8px;border:1px solid var(--border-color);background:var(--bg-secondary);color:var(--text-primary);font-size:0.95rem;font-family:inherit;resize:vertical;"><?php echo htmlspecialchars($form['message']); ?></textarea>
                    </div>

                    <button type="submit"
                        style="display:inline-block;padding:12px 32px;border-radius:8px;font-weight:700;font-size:0.95rem;border:none;cursor:pointer;background:linear-gradient(135deg,#dc2626,#ef4444);color:white;box-shadow:0 4px 15px rgba(220,38,38,0.3);transition:all 0.3s ease;font-family:inherit;">
                        Send Enquiry
                    </button>
                    <a href="enterprise.php" style="margin-left:1rem;color:var(--text-muted);font-size:0.9rem;">Cancel</a>
                </form>

            <?php endif; ?>
        </div>

        <div class="page-nav">
            <a href="enterprise.php" class="prev">&larr; Enterprise Plans</a>
        </div>

    </main>
<?php include 'includes/footer.php'; ?>

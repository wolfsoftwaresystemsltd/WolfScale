<?php
/**
 * SMTP mailer — sends via smtp.stackmail.com:587 with STARTTLS.
 * Returns true on success, error string on failure.
 */
function smtp_send(string $to, string $subject, string $body, string $reply_to): string|bool {
    $host = 'smtp.stackmail.com';
    $port = 587;
    $user = 'noreply@wolfstack.org';
    $pass = 'Lt24bf283';
    $from = 'noreply@wolfstack.org';
    $from_name = 'WolfStack';

    $sock = @fsockopen($host, $port, $errno, $errstr, 10);
    if (!$sock) return "Connection failed: $errstr ($errno)";

    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '220')) return "Bad greeting: $resp";

    smtp_write($sock, "EHLO wolfstack.org\r\n");
    $resp = smtp_read($sock);

    smtp_write($sock, "STARTTLS\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '220')) return "STARTTLS rejected: $resp";

    $crypto = stream_socket_enable_crypto($sock, true, STREAM_CRYPTO_METHOD_TLSv1_2_CLIENT | STREAM_CRYPTO_METHOD_TLSv1_3_CLIENT);
    if (!$crypto) return 'TLS handshake failed';

    smtp_write($sock, "EHLO wolfstack.org\r\n");
    $resp = smtp_read($sock);

    smtp_write($sock, "AUTH LOGIN\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '334')) return "AUTH rejected: $resp";

    smtp_write($sock, base64_encode($user) . "\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '334')) return "Username rejected: $resp";

    smtp_write($sock, base64_encode($pass) . "\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '235')) return "Auth failed: $resp";

    smtp_write($sock, "MAIL FROM:<$from>\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '250')) return "MAIL FROM rejected: $resp";

    smtp_write($sock, "RCPT TO:<$to>\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '250')) return "RCPT TO rejected: $resp";

    smtp_write($sock, "DATA\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '354')) return "DATA rejected: $resp";

    $date = date('r');
    $msg_id = '<' . uniqid('ws-', true) . '@wolfstack.org>';
    $subject_enc = '=?UTF-8?B?' . base64_encode($subject) . '?=';

    $headers  = "Date: $date\r\n";
    $headers .= "From: $from_name <$from>\r\n";
    $headers .= "Reply-To: $reply_to\r\n";
    $headers .= "To: $to\r\n";
    $headers .= "Subject: $subject_enc\r\n";
    $headers .= "Message-ID: $msg_id\r\n";
    $headers .= "MIME-Version: 1.0\r\n";
    $headers .= "Content-Type: text/plain; charset=UTF-8\r\n";
    $headers .= "Content-Transfer-Encoding: 8bit\r\n";
    $headers .= "\r\n";

    // Dot-stuff the body (lines starting with . get an extra .)
    $body = str_replace("\r\n.", "\r\n..", $body);

    smtp_write($sock, $headers . $body . "\r\n.\r\n");
    $resp = smtp_read($sock);
    if (!str_starts_with($resp, '250')) return "Message rejected: $resp";

    smtp_write($sock, "QUIT\r\n");
    smtp_read($sock);
    fclose($sock);

    return true;
}

function smtp_write($sock, string $data): void {
    fwrite($sock, $data);
}

function smtp_read($sock): string {
    $response = '';
    while ($line = fgets($sock, 512)) {
        $response .= $line;
        // Multi-line responses have - after code; last line has space
        if (isset($line[3]) && $line[3] === ' ') break;
        if (strlen($line) < 4) break;
    }
    return trim($response);
}

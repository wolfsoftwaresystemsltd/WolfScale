<?php
/**
 * WolfScale Connection Test
 * Lists all tables in the crm_1 database
 */

$host = 'yourserver';
$port = 8007;  // WolfScale MySQL proxy port
$database = 'your_database';
$user = 'your_username';  // <-- Update this
$password = 'your_password';  // <-- Update this

echo "Connecting to WolfScale at $host:$port...\n";

try {
    // Connect using PDO
    $dsn = "mysql:host=$host;port=$port;dbname=$database;charset=utf8mb4";
    $options = [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
        PDO::ATTR_TIMEOUT => 5,
    ];
    
    $pdo = new PDO($dsn, $user, $password, $options);
    
    echo "✓ Connected successfully!\n\n";
    
    // Get list of tables
    $stmt = $pdo->query("SHOW TABLES");
    $tables = $stmt->fetchAll(PDO::FETCH_COLUMN);
    
    if (empty($tables)) {
        echo "No tables found in database '$database'\n";
    } else {
        echo "Tables in '$database':\n";
        echo str_repeat('-', 40) . "\n";
        foreach ($tables as $table) {
            echo "  • $table\n";
        }
        echo str_repeat('-', 40) . "\n";
        echo "Total: " . count($tables) . " table(s)\n";
    }
    
} catch (PDOException $e) {
    echo "✗ Connection failed!\n";
    echo "Error: " . $e->getMessage() . "\n";
    
    // Common troubleshooting tips
    echo "\nTroubleshooting:\n";
    echo "  1. Is WolfScale running? Check: curl http://$host:8080/cluster\n";
    echo "  2. Is port $port open? Check: nc -zv $host $port\n";
    echo "  3. Are credentials correct?\n";
    echo "  4. Does the database '$database' exist?\n";
    exit(1);
}

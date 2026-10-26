//! Importers / exporters: OpenSSH config, PuTTY, MobaXterm, SecureCRT, CSV.

use std::collections::HashMap;

use uuid::Uuid;

use crate::forward::parse_forward_spec;
use crate::model::{ForwardKind, Group, Host, PortForward, Protocol, SshKey, Vault};

#[derive(Default)]
pub struct ImportResult {
    pub hosts: Vec<Host>,
    pub groups: Vec<Group>,
    pub keys: Vec<SshKey>,
    pub forwards: Vec<PortForward>,
    pub warnings: Vec<String>,
}

impl ImportResult {
    pub fn summary(&self) -> String {
        format!(
            "Imported {} hosts, {} groups, {} keys, {} port forwarding rules{}",
            self.hosts.len(),
            self.groups.len(),
            self.keys.len(),
            self.forwards.len(),
            if self.warnings.is_empty() {
                String::new()
            } else {
                format!(" ({} warnings)", self.warnings.len())
            }
        )
    }

    /// Merge into a vault, skipping hosts that already exist (same address/port/user).
    pub fn apply(self, v: &mut Vault) -> String {
        let summary = self.summary();
        for g in self.groups {
            if !v
                .groups
                .iter()
                .any(|x| x.name == g.name && x.parent_id == g.parent_id)
            {
                v.groups.push(g);
            }
        }
        for k in self.keys {
            if !v.keys.iter().any(|x| x.public_key == k.public_key) {
                v.keys.push(k);
            }
        }
        for h in self.hosts {
            let dup = v.hosts.iter().any(|x| {
                x.address == h.address
                    && x.settings.port == h.settings.port
                    && x.settings.username == h.settings.username
                    && x.label == h.label
            });
            if !dup {
                v.hosts.push(h);
            }
        }
        v.forwards.extend(self.forwards);
        summary
    }

    fn group_id(&mut self, name: &str) -> Option<Uuid> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut parent: Option<Uuid> = None;
        for part in name.split(['/', '\\']).filter(|p| !p.trim().is_empty()) {
            let part = part.trim();
            let existing = self
                .groups
                .iter()
                .find(|g| g.name == part && g.parent_id == parent)
                .map(|g| g.id);
            parent = Some(match existing {
                Some(id) => id,
                None => {
                    let g = Group {
                        name: part.to_string(),
                        parent_id: parent,
                        ..Default::default()
                    };
                    let id = g.id;
                    self.groups.push(g);
                    id
                }
            });
        }
        parent
    }
}

// ---------------------------------------------------------------------------
// OpenSSH config
// ---------------------------------------------------------------------------

pub fn parse_ssh_config(text: &str, existing: &Vault) -> ImportResult {
    let mut res = ImportResult::default();
    let mut blocks: Vec<(Vec<String>, Vec<(String, String)>)> = vec![];
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (
                k.trim().to_lowercase(),
                v.trim()
                    .trim_start_matches('=')
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            ),
            None => continue,
        };
        if k == "host" {
            blocks.push((
                v.split_whitespace().map(|s| s.to_string()).collect(),
                vec![],
            ));
        } else if k == "match" {
            blocks.push((vec![], vec![]));
        } else if let Some(b) = blocks.last_mut() {
            b.1.push((k, v));
        }
    }
    let mut alias_to_id: HashMap<String, Uuid> = existing
        .hosts
        .iter()
        .map(|h| (h.label.clone(), h.id))
        .collect();
    let mut pending_jumps: Vec<(Uuid, String)> = vec![];
    for (patterns, opts) in blocks {
        for alias in patterns
            .iter()
            .filter(|p| !p.contains('*') && !p.contains('?') && !p.starts_with('!'))
        {
            let mut h = Host {
                label: alias.clone(),
                address: alias.clone(),
                ..Default::default()
            };
            for (k, v) in &opts {
                match k.as_str() {
                    "hostname" => h.address = v.clone(),
                    "user" => h.settings.username = Some(v.clone()),
                    "port" => h.settings.port = v.parse().ok(),
                    "forwardagent" => {
                        h.settings.agent_forwarding = Some(v.eq_ignore_ascii_case("yes"))
                    }
                    "serveraliveinterval" => h.settings.keepalive_secs = v.parse().ok(),
                    "proxyjump" => pending_jumps.push((h.id, v.clone())),
                    "proxycommand" if !v.eq_ignore_ascii_case("none") => {
                        h.settings.proxy = Some(crate::model::ProxyConfig {
                            kind: crate::model::ProxyKind::Command,
                            command: v.clone(),
                            ..Default::default()
                        })
                    }
                    "identityfile" => {
                        let path = expand_tilde(v);
                        match std::fs::read_to_string(&path) {
                            Ok(text) => {
                                match crate::keys::import(&format!("{alias} key"), &text, "") {
                                    Ok(k) => {
                                        let id = match res
                                            .keys
                                            .iter()
                                            .find(|x| x.public_key == k.public_key)
                                        {
                                            Some(x) => x.id,
                                            None => {
                                                let id = existing
                                                    .keys
                                                    .iter()
                                                    .find(|x| x.public_key == k.public_key)
                                                    .map(|x| x.id)
                                                    .unwrap_or(k.id);
                                                if id == k.id {
                                                    res.keys.push(k);
                                                }
                                                id
                                            }
                                        };
                                        h.settings.key_id = Some(id);
                                    }
                                    Err(_) => {
                                        // Encrypted keys: import raw so the passphrase is asked on connect.
                                        let mut k = SshKey {
                                            id: Uuid::new_v4(),
                                            label: format!("{alias} key"),
                                            private_key: text.clone(),
                                            key_type: "encrypted".into(),
                                            updated: crate::model::now_ts(),
                                            ..Default::default()
                                        };
                                        if let Ok(p) =
                                            std::fs::read_to_string(format!("{path}.pub"))
                                        {
                                            k.public_key = p.trim().to_string();
                                        }
                                        h.settings.key_id = Some(k.id);
                                        res.keys.push(k);
                                    }
                                }
                            }
                            Err(_) => res.warnings.push(format!("{alias}: cannot read {path}")),
                        }
                    }
                    "localforward" | "remoteforward" | "dynamicforward" => {
                        let kind = match k.as_str() {
                            "localforward" => ForwardKind::Local,
                            "remoteforward" => ForwardKind::Remote,
                            _ => ForwardKind::Dynamic,
                        };
                        let spec = v.split_whitespace().collect::<Vec<_>>().join(":");
                        if let Some(mut r) = parse_forward_spec(&spec, kind) {
                            r.host_id = Some(h.id);
                            r.label = format!("{alias} {}", kind.label());
                            res.forwards.push(r);
                        }
                    }
                    _ => {}
                }
            }
            alias_to_id.insert(alias.clone(), h.id);
            res.hosts.push(h);
        }
    }
    for (host_id, jumps) in pending_jumps {
        let mut chain = vec![];
        for j in jumps.split(',') {
            let j = j.trim();
            let alias = j
                .rsplit('@')
                .next()
                .unwrap_or(j)
                .split(':')
                .next()
                .unwrap_or(j);
            if let Some(id) = alias_to_id.get(alias) {
                chain.push(*id);
            } else {
                // Create an ad-hoc jump host.
                let (user, host, port) =
                    crate::model::parse_quick_connect(j).unwrap_or((None, j.to_string(), None));
                let mut jh = Host {
                    label: host.clone(),
                    address: host.clone(),
                    ..Default::default()
                };
                jh.settings.username = user;
                jh.settings.port = port;
                chain.push(jh.id);
                alias_to_id.insert(host, jh.id);
                res.hosts.push(jh);
            }
        }
        if let Some(h) = res.hosts.iter_mut().find(|h| h.id == host_id) {
            h.settings.host_chain = chain;
        }
    }
    res
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = directories::UserDirs::new() {
            return home.home_dir().join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

pub fn export_ssh_config(v: &Vault) -> String {
    let mut out = String::from("# Exported by Ferrum\n");
    for h in v.hosts.iter().filter(|h| h.protocol == Protocol::Ssh) {
        let r = v.resolve(h);
        let alias: String = h
            .display_name()
            .chars()
            .map(|c| if c.is_whitespace() { '-' } else { c })
            .collect();
        out.push_str(&format!(
            "\nHost {alias}\n    HostName {}\n    Port {}\n",
            r.address, r.port
        ));
        if !r.username.is_empty() {
            out.push_str(&format!("    User {}\n", r.username));
        }
        if r.agent_forwarding {
            out.push_str("    ForwardAgent yes\n");
        }
        if r.keepalive_secs > 0 {
            out.push_str(&format!("    ServerAliveInterval {}\n", r.keepalive_secs));
        }
        if !r.chain.is_empty() {
            let j: Vec<String> = r
                .chain
                .iter()
                .map(|c| {
                    if c.username.is_empty() {
                        format!("{}:{}", c.address, c.port)
                    } else {
                        format!("{}@{}:{}", c.username, c.address, c.port)
                    }
                })
                .collect();
            out.push_str(&format!("    ProxyJump {}\n", j.join(",")));
        }
        for f in v.forwards.iter().filter(|f| f.host_id == Some(h.id)) {
            match f.kind {
                ForwardKind::Local => out.push_str(&format!(
                    "    LocalForward {}:{} {}:{}\n",
                    f.bind_address, f.bind_port, f.dest_host, f.dest_port
                )),
                ForwardKind::Remote => out.push_str(&format!(
                    "    RemoteForward {}:{} {}:{}\n",
                    f.bind_address, f.bind_port, f.dest_host, f.dest_port
                )),
                ForwardKind::Dynamic => out.push_str(&format!(
                    "    DynamicForward {}:{}\n",
                    f.bind_address, f.bind_port
                )),
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// PuTTY (.reg export of HKCU\Software\SimonTatham\PuTTY\Sessions)
// ---------------------------------------------------------------------------

fn putty_unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

pub fn parse_putty_reg(text: &str) -> ImportResult {
    let mut res = ImportResult::default();
    let text = text.replace('\0', "");
    let mut cur: Option<(String, HashMap<String, String>)> = None;
    let mut sessions = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            if let Some(c) = cur.take() {
                sessions.push(c);
            }
            if let Some(idx) = line.find("\\Sessions\\") {
                let name = putty_unescape(line[idx + 10..].trim_end_matches(']'));
                cur = Some((name, HashMap::new()));
            }
        } else if let Some((_, map)) = cur.as_mut() {
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim_matches('"').to_string();
                let v = if let Some(d) = v.strip_prefix("dword:") {
                    u32::from_str_radix(d, 16)
                        .map(|n| n.to_string())
                        .unwrap_or_default()
                } else {
                    v.trim_matches('"').replace("\\\\", "\\")
                };
                map.insert(k, v);
            }
        }
    }
    if let Some(c) = cur.take() {
        sessions.push(c);
    }
    for (name, m) in sessions {
        if name == "Default Settings" {
            continue;
        }
        let proto = match m.get("Protocol").map(|s| s.as_str()) {
            Some("telnet") => Protocol::Telnet,
            Some("serial") => Protocol::Serial,
            _ => Protocol::Ssh,
        };
        let mut hostname = m.get("HostName").cloned().unwrap_or_default();
        let mut h = Host {
            label: name.clone(),
            protocol: proto,
            ..Default::default()
        };
        if let Some((u, host)) = hostname.clone().split_once('@') {
            h.settings.username = Some(u.to_string());
            hostname = host.to_string();
        }
        h.address = hostname;
        if let Some(u) = m.get("UserName").filter(|u| !u.is_empty()) {
            h.settings.username = Some(u.clone());
        }
        h.settings.port = m.get("PortNumber").and_then(|p| p.parse().ok());
        if m.get("AgentFwd").map(|s| s == "1").unwrap_or(false) {
            h.settings.agent_forwarding = Some(true);
        }
        if proto == Protocol::Serial {
            h.serial.device = m.get("SerialLine").cloned().unwrap_or_default();
            h.serial.baud_rate = m
                .get("SerialSpeed")
                .and_then(|s| s.parse().ok())
                .unwrap_or(9600);
            h.address = h.serial.device.clone();
        }
        if let Some(k) = m.get("PublicKeyFile").filter(|k| !k.is_empty()) {
            res.warnings.push(format!(
                "{name}: PuTTY key file {k} must be imported separately (Keychain → Import)"
            ));
        }
        if h.address.is_empty() {
            continue;
        }
        res.hosts.push(h);
    }
    res
}

// ---------------------------------------------------------------------------
// MobaXterm (.mxtsessions / MobaXterm.ini [Bookmarks*] sections)
// ---------------------------------------------------------------------------

pub fn parse_mobaxterm(text: &str) -> ImportResult {
    let mut res = ImportResult::default();
    let mut folder = String::new();
    let mut in_bookmarks = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_bookmarks = line.to_lowercase().starts_with("[bookmarks");
            folder.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name == "SubRep" {
            folder = value.replace('\\', "/");
            continue;
        }
        if name == "ImgNum" || !value.contains('%') {
            continue;
        }
        // Format: #icon#type%host%port%user%...
        let payload = value.split_once('#').map(|x| x.1).unwrap_or(value);
        let payload = payload.split_once('#').map(|x| x.1).unwrap_or(payload);
        let f: Vec<&str> = payload.split('%').collect();
        if f.len() < 4 {
            continue;
        }
        let proto = match f[0] {
            "0" => Protocol::Ssh,
            "1" => Protocol::Telnet,
            "8" => Protocol::Mosh,
            "10" => Protocol::Serial,
            _ => continue,
        };
        let mut h = Host {
            label: name.trim().to_string(),
            protocol: proto,
            address: f[1].to_string(),
            ..Default::default()
        };
        h.settings.port = f[2].parse().ok();
        if !f[3].is_empty() {
            h.settings.username = Some(f[3].to_string());
        }
        let gid = res.group_id(&folder);
        h.group_id = gid;
        if !h.address.is_empty() {
            res.hosts.push(h);
        }
    }
    res
}

// ---------------------------------------------------------------------------
// SecureCRT XML export
// ---------------------------------------------------------------------------

pub fn parse_securecrt_xml(text: &str) -> ImportResult {
    let mut res = ImportResult::default();
    // Stack of (key name, values) for nested <key> elements.
    let mut stack: Vec<(String, HashMap<String, String>)> = vec![];
    let tag_re =
        regex::Regex::new(r#"<(/?)(key|string|dword)\s*(?:name="([^"]*)")?\s*(/?)>([^<]*)"#)
            .unwrap();
    for cap in tag_re.captures_iter(text) {
        let closing = &cap[1] == "/";
        let kind = &cap[2];
        let name = cap.get(3).map(|m| m.as_str()).unwrap_or("");
        let self_close = &cap[4] == "/";
        let value = cap.get(5).map(|m| m.as_str().trim()).unwrap_or("");
        match (kind, closing) {
            ("key", false) if !self_close => stack.push((name.to_string(), HashMap::new())),
            ("key", true) => {
                let Some((session_name, m)) = stack.pop() else {
                    continue;
                };
                let Some(host) = m.get("Hostname").filter(|h| !h.is_empty()) else {
                    continue;
                };
                let mut h = Host {
                    label: session_name,
                    address: host.clone(),
                    ..Default::default()
                };
                h.settings.username = m.get("Username").filter(|u| !u.is_empty()).cloned();
                h.settings.port = m
                    .get("[SSH2] Port")
                    .or(m.get("Port"))
                    .and_then(|p| p.parse().ok());
                if m.get("Protocol Name")
                    .map(|p| p.eq_ignore_ascii_case("telnet"))
                    .unwrap_or(false)
                {
                    h.protocol = Protocol::Telnet;
                }
                let path: Vec<String> = stack
                    .iter()
                    .map(|(n, _)| n.clone())
                    .filter(|n| {
                        !n.is_empty() && n != "Sessions" && n != "VanDyke" && n != "SecureCRT"
                    })
                    .collect();
                let gid = res.group_id(&path.join("/"));
                h.group_id = gid;
                res.hosts.push(h);
            }
            ("string", false) | ("dword", false) => {
                if let Some((_, m)) = stack.last_mut() {
                    let v = value
                        .replace("&amp;", "&")
                        .replace("&lt;", "<")
                        .replace("&gt;", ">");
                    m.insert(name.to_string(), v);
                }
            }
            _ => {}
        }
    }
    res
}

// ---------------------------------------------------------------------------
// CSV host import and export
// ---------------------------------------------------------------------------

fn csv_split(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if q && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => q = !q,
            ',' if !q => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub const CSV_HEADER: &str = "Groups,Label,Tags,Hostname/IP,Protocol,Port,Username,Password";

pub fn parse_csv(text: &str) -> ImportResult {
    let mut res = ImportResult::default();
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .map(csv_split)
        .unwrap_or_default()
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();
    let col = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h == n));
    let (g, l, t, a, p, port, u, pw) = (
        col(&["groups", "group", "folder"]),
        col(&["label", "name", "alias"]),
        col(&["tags", "tag"]),
        col(&["hostname/ip", "hostname", "host", "address", "ip"]),
        col(&["protocol"]),
        col(&["port"]),
        col(&["username", "user"]),
        col(&["password"]),
    );
    let Some(a) = a else {
        res.warnings.push("CSV needs a Hostname/IP column".into());
        return res;
    };
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let f = csv_split(line);
        let get = |i: Option<usize>| {
            i.and_then(|i| f.get(i))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let mut h = Host {
            address: get(Some(a)),
            label: get(l),
            ..Default::default()
        };
        if h.address.is_empty() {
            continue;
        }
        h.protocol = match get(p).to_lowercase().as_str() {
            "telnet" => Protocol::Telnet,
            "mosh" => Protocol::Mosh,
            _ => Protocol::Ssh,
        };
        h.settings.port = get(port).parse().ok();
        h.settings.username = Some(get(u)).filter(|s| !s.is_empty());
        h.settings.password = Some(get(pw)).filter(|s| !s.is_empty());
        h.tags = get(t)
            .split([';', '|'])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let gid = res.group_id(&get(g));
        h.group_id = gid;
        res.hosts.push(h);
    }
    res
}

pub fn export_csv(v: &Vault, include_passwords: bool) -> String {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for h in &v.hosts {
        let r = v.resolve(h);
        let row = [
            v.group_path(h.group_id),
            h.label.clone(),
            h.tags.join(";"),
            h.address.clone(),
            h.protocol.label().to_lowercase(),
            r.port.to_string(),
            r.username.clone(),
            if include_passwords {
                r.password.clone().unwrap_or_default()
            } else {
                String::new()
            },
        ];
        out.push_str(
            &row.iter()
                .map(|c| csv_escape(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_hosts_jump_and_forwards() {
        let cfg = "Host bastion\n  HostName 1.2.3.4\n  User ops\n\nHost app web\n  HostName 10.0.0.5\n  Port 2222\n  ProxyJump bastion\n  LocalForward 8080 localhost:80\n  ForwardAgent yes\n\nHost *\n  ServerAliveInterval 30\n";
        let r = parse_ssh_config(cfg, &Vault::default());
        assert_eq!(r.hosts.len(), 3);
        let app = r.hosts.iter().find(|h| h.label == "app").unwrap();
        let bastion = r.hosts.iter().find(|h| h.label == "bastion").unwrap();
        assert_eq!(app.settings.port, Some(2222));
        assert_eq!(app.settings.host_chain, vec![bastion.id]);
        assert_eq!(app.settings.agent_forwarding, Some(true));
        assert_eq!(r.forwards.len(), 2);
        assert_eq!(r.forwards[0].dest_port, 80);
    }

    #[test]
    fn csv_roundtrip_with_groups_and_quotes() {
        let csv = "Groups,Label,Tags,Hostname/IP,Protocol,Port,Username,Password\nProd/DB,\"db, primary\",sql;prod,10.1.1.1,ssh,22,root,\n";
        let r = parse_csv(csv);
        assert_eq!(r.hosts.len(), 1);
        assert_eq!(r.hosts[0].label, "db, primary");
        assert_eq!(r.hosts[0].tags, vec!["sql", "prod"]);
        assert_eq!(r.groups.len(), 2);
        let mut v = Vault::default();
        r.apply(&mut v);
        let out = export_csv(&v, false);
        assert!(out.contains("Prod / DB,\"db, primary\",sql;prod,10.1.1.1,ssh,22,root,"));
    }

    #[test]
    fn putty_reg_sessions() {
        let reg = "[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\my%20server]\n\"HostName\"=\"admin@example.com\"\n\"PortNumber\"=dword:00000016\n\"Protocol\"=\"ssh\"\n";
        let r = parse_putty_reg(reg);
        assert_eq!(r.hosts.len(), 1);
        assert_eq!(r.hosts[0].label, "my server");
        assert_eq!(r.hosts[0].address, "example.com");
        assert_eq!(r.hosts[0].settings.username.as_deref(), Some("admin"));
        assert_eq!(r.hosts[0].settings.port, Some(22));
    }

    #[test]
    fn securecrt_xml_sessions_with_folders() {
        let xml = r#"<VanDyke version="3.0"><key name="Sessions"><key name="Prod"><key name="api"><string name="Hostname">10.9.9.9</string><string name="Username">svc</string><dword name="[SSH2] Port">2200</dword></key></key></key></VanDyke>"#;
        let r = parse_securecrt_xml(xml);
        assert_eq!(r.hosts.len(), 1);
        assert_eq!(r.hosts[0].label, "api");
        assert_eq!(r.hosts[0].settings.port, Some(2200));
        assert_eq!(r.groups[0].name, "Prod");
    }

    #[test]
    fn mobaxterm_bookmarks() {
        let ini = "[Bookmarks_1]\nSubRep=Servers\\Web\nImgNum=41\nweb1=#109#0%192.168.1.10%22%deploy%%-1%-1%%%22%%0%0%0%%%-1%0%0%0%%1080%%0%0%1\n";
        let r = parse_mobaxterm(ini);
        assert_eq!(r.hosts.len(), 1);
        assert_eq!(r.hosts[0].address, "192.168.1.10");
        assert_eq!(r.hosts[0].settings.username.as_deref(), Some("deploy"));
        assert_eq!(r.groups.len(), 2);
    }
}

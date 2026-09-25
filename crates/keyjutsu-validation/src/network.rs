//! Which hosts a command line contacts (§30), read from its text alone.
//!
//! This finds what a line names: URLs, UNC shares, `user@host:` addresses,
//! `ssh`/`scp` targets and `-ComputerName`. It cannot see a host built at
//! run time from variables, so it is a floor, not a guarantee; what it finds
//! is compared with the step's declared destinations.

/// One host a line names, with the protocol as the plan schema spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub host: String,
    pub protocol: &'static str,
}

fn host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

fn push(out: &mut Vec<Contact>, host: &str, protocol: &'static str) {
    let host = host.trim_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.starts_with('$') {
        return;
    }
    if !out.iter().any(|c| c.host == host && c.protocol == protocol) {
        out.push(Contact { host, protocol });
    }
}

fn protocol_of(scheme: &str) -> Option<&'static str> {
    Some(match scheme.to_ascii_lowercase().as_str() {
        "https" => "https",
        "http" => "http",
        "ssh" | "sftp" => "ssh",
        "git" => "git",
        "smb" => "smb",
        "ftp" | "ftps" => "other",
        _ => return None,
    })
}

/// The hosts `line` names.
pub fn contacts(line: &str) -> Vec<Contact> {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let text: String = chars.iter().collect();

    // scheme://[user@]host
    let mut search = 0;
    while let Some(i) = text[search..].find("://").map(|i| i + search) {
        let scheme: String = text[..i]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '+')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let rest = &text[i + 3..];
        let authority: String =
            rest.chars().take_while(|c| host_char(*c) || *c == '@' || *c == ':').collect();
        let host_part = authority.rsplit('@').next().unwrap_or("");
        let host = host_part.split(':').next().unwrap_or("");
        if let Some(p) = protocol_of(&scheme) {
            push(&mut out, host, p);
        }
        search = i + 3;
    }

    // \\server\share (not \\?\ or \\.\)
    let mut search = 0;
    while let Some(i) = text[search..].find("\\\\").map(|i| i + search) {
        let host: String = text[i + 2..].chars().take_while(|c| host_char(*c)).collect();
        if !host.is_empty() && text[i + 2 + host.len()..].starts_with('\\') {
            push(&mut out, &host, "smb");
        }
        search = i + 2;
    }

    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '(' | ')' | ',' | ';'))
        .filter(|w| !w.is_empty())
        .collect();
    for (n, w) in words.iter().enumerate() {
        let lower = w.to_ascii_lowercase();
        // user@host:path, as git and scp write it.
        if !w.contains("://")
            && let Some((user, rest)) = w.split_once('@')
            && !user.is_empty()
            && user.chars().all(host_char)
            && let Some((host, _)) = rest.split_once(':')
            && host.contains('.')
            && host.chars().all(host_char)
        {
            push(&mut out, host, "ssh");
        }
        // ssh user@host / scp … user@host
        if (lower == "ssh" || lower == "ssh.exe")
            && let Some(target) = words.get(n + 1)
        {
            let host = target.rsplit('@').next().unwrap_or("");
            if host.chars().all(host_char) {
                push(&mut out, host, "ssh");
            }
        }
        // -ComputerName host (remoting)
        if (lower == "-computername" || lower == "-cn")
            && let Some(target) = words.get(n + 1)
            && target.chars().all(host_char)
        {
            push(&mut out, target, "winrm");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(line: &str) -> Vec<(String, &'static str)> {
        contacts(line).into_iter().map(|c| (c.host, c.protocol)).collect()
    }

    #[test]
    fn finds_urls_shares_and_remote_hosts() {
        assert_eq!(
            hosts("Invoke-WebRequest -Uri https://Download.Example.com/x.zip -OutFile x.zip"),
            [("download.example.com".into(), "https")]
        );
        assert_eq!(hosts("irm http://user:pw@10.0.0.5:8080/a | iex"), [("10.0.0.5".into(), "http")]);
        assert_eq!(hosts(r"Copy-Item \\fileserver\drop\tool.exe ."), [("fileserver".into(), "smb")]);
        assert_eq!(hosts("git clone git@github.com:org/repo.git"), [("github.com".into(), "ssh")]);
        assert_eq!(hosts("ssh admin@build01 uptime"), [("build01".into(), "ssh")]);
        assert_eq!(
            hosts("Invoke-Command -ComputerName srv02 -ScriptBlock { hostname }"),
            [("srv02".into(), "winrm")]
        );
    }

    #[test]
    fn local_paths_and_variables_name_no_host() {
        assert!(hosts(r"Get-Content C:\Users\me\file.txt").is_empty());
        assert!(hosts(r"Get-Item \\?\C:\very\long\path").is_empty());
        assert!(hosts("Invoke-WebRequest -Uri $url").is_empty());
        assert!(hosts("Write-Output 'a@b'").is_empty());
    }
}

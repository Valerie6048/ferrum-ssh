//! Import servers from cloud providers: DigitalOcean, AWS EC2 and Azure.

use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::model::{Host, Settings};

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Ferrum-SSH")
        .build()
        .expect("http client")
}

fn host(label: String, address: String, tag: &str, user: Option<&str>) -> Host {
    let mut h = Host {
        label,
        address,
        tags: vec![tag.to_string()],
        ..Default::default()
    };
    h.settings.username = user.map(|s| s.to_string());
    h
}

pub async fn digitalocean(token: &str) -> Result<Vec<Host>> {
    if token.trim().is_empty() {
        bail!("add a DigitalOcean API token in Settings → Integrations");
    }
    let mut out = vec![];
    let mut url = "https://api.digitalocean.com/v2/droplets?per_page=200".to_string();
    loop {
        let v: serde_json::Value = http()
            .get(&url)
            .bearer_auth(token.trim())
            .send()
            .await?
            .error_for_status()
            .context("DigitalOcean API")?
            .json()
            .await?;
        for d in v["droplets"].as_array().cloned().unwrap_or_default() {
            let ip = d["networks"]["v4"]
                .as_array()
                .and_then(|a| a.iter().find(|n| n["type"] == "public"))
                .and_then(|n| n["ip_address"].as_str())
                .unwrap_or_default()
                .to_string();
            if ip.is_empty() {
                continue;
            }
            let mut h = host(
                d["name"].as_str().unwrap_or("droplet").to_string(),
                ip,
                "digitalocean",
                Some("root"),
            );
            h.os = d["image"]["distribution"].as_str().map(|s| s.to_string());
            out.push(h);
        }
        match v["links"]["pages"]["next"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------------------
// AWS EC2 (Query API, Signature V4)
// ----------------------------------------------------------------------------

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub fn sigv4_headers(
    access: &str,
    secret: &str,
    region: &str,
    service: &str,
    host: &str,
    query: &str,
    amz_date: &str,
) -> (String, String) {
    let date = &amz_date[..8];
    let payload_hash = hex::encode(Sha256::digest(b""));
    let canonical_headers = format!("host:{host}\nx-amz-date:{amz_date}\n");
    let signed_headers = "host;x-amz-date";
    let canonical_request =
        format!("GET\n/\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let k_date = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    (
        format!(
            "AWS4-HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
        payload_hash,
    )
}

pub async fn aws(s: &Settings) -> Result<Vec<Host>> {
    if s.aws_access_key.trim().is_empty() || s.aws_secret_key.trim().is_empty() {
        bail!("add AWS access keys in Settings → Integrations");
    }
    let ip_re = regex::Regex::new(r"<ipAddress>([^<]+)</ipAddress>").unwrap();
    let dns_re = regex::Regex::new(r"<dnsName>([^<]+)</dnsName>").unwrap();
    let name_re = regex::Regex::new(r"(?s)<key>Name</key>\s*<value>([^<]*)</value>").unwrap();
    let plat_re = regex::Regex::new(r"<platformDetails>([^<]+)</platformDetails>").unwrap();
    let mut out = vec![];
    for region in s
        .aws_regions
        .split([',', ' '])
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        let host_name = format!("ec2.{region}.amazonaws.com");
        let query = "Action=DescribeInstances&Version=2016-11-15";
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let (auth, _) = sigv4_headers(
            s.aws_access_key.trim(),
            s.aws_secret_key.trim(),
            region,
            "ec2",
            &host_name,
            query,
            &amz_date,
        );
        let body = http()
            .get(format!("https://{host_name}/?{query}"))
            .header("x-amz-date", &amz_date)
            .header("Authorization", auth)
            .send()
            .await?
            .text()
            .await?;
        if body.contains("<Errors>") {
            let msg = regex::Regex::new(r"<Message>([^<]+)</Message>")
                .unwrap()
                .captures(&body)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| "request failed".into());
            bail!("AWS {region}: {msg}");
        }
        // Each instance starts at an <instanceId>; scan until the next one.
        for seg in body.split("<instanceId>").skip(1) {
            let Some((id, rest)) = seg.split_once("</instanceId>") else {
                continue;
            };
            let id = id.to_string();
            let addr = ip_re
                .captures(rest)
                .map(|c| c[1].to_string())
                .or_else(|| dns_re.captures(rest).map(|c| c[1].to_string()));
            let Some(addr) = addr else { continue };
            let name = name_re
                .captures(rest)
                .map(|c| c[1].to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or(id);
            let platform = plat_re.captures(rest).map(|c| c[1].to_string());
            let user = match platform.as_deref() {
                Some(p) if p.to_lowercase().contains("ubuntu") => "ubuntu",
                Some(p) if p.to_lowercase().contains("windows") => "Administrator",
                _ => "ec2-user",
            };
            let mut h = host(name, addr, "aws", Some(user));
            h.os = platform;
            h.tags.push(region.to_string());
            out.push(h);
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------------------
// Azure (service principal, ARM REST)
// ----------------------------------------------------------------------------

pub async fn azure(s: &Settings) -> Result<Vec<Host>> {
    if s.azure_tenant.is_empty()
        || s.azure_client_id.is_empty()
        || s.azure_client_secret.is_empty()
        || s.azure_subscription.is_empty()
    {
        bail!("add Azure tenant, client id, secret and subscription in Settings → Integrations");
    }
    let token: serde_json::Value = http()
        .post(format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            s.azure_tenant.trim()
        ))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", s.azure_client_id.trim()),
            ("client_secret", s.azure_client_secret.trim()),
            ("scope", "https://management.azure.com/.default"),
        ])
        .send()
        .await?
        .error_for_status()
        .context("Azure login")?
        .json()
        .await?;
    let bearer = token["access_token"]
        .as_str()
        .context("no access token")?
        .to_string();
    let base = format!(
        "https://management.azure.com/subscriptions/{}/providers",
        s.azure_subscription.trim()
    );
    let get = |url: String| {
        let b = bearer.clone();
        async move {
            let v: serde_json::Value = http()
                .get(url)
                .bearer_auth(b)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            anyhow::Ok(v)
        }
    };
    let vms = get(format!(
        "{base}/Microsoft.Compute/virtualMachines?api-version=2024-07-01"
    ))
    .await?;
    let nics = get(format!(
        "{base}/Microsoft.Network/networkInterfaces?api-version=2024-05-01"
    ))
    .await?;
    let ips = get(format!(
        "{base}/Microsoft.Network/publicIPAddresses?api-version=2024-05-01"
    ))
    .await?;
    let lower = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_lowercase();
    let mut out = vec![];
    for vm in vms["value"].as_array().cloned().unwrap_or_default() {
        let name = vm["name"].as_str().unwrap_or("vm").to_string();
        let user = vm["properties"]["osProfile"]["adminUsername"]
            .as_str()
            .map(|s| s.to_string());
        let nic_ids: Vec<String> = vm["properties"]["networkProfile"]["networkInterfaces"]
            .as_array()
            .map(|a| a.iter().map(|n| lower(&n["id"])).collect())
            .unwrap_or_default();
        let mut addr = None;
        for nic in nics["value"].as_array().into_iter().flatten() {
            if !nic_ids.contains(&lower(&nic["id"])) {
                continue;
            }
            for cfg in nic["properties"]["ipConfigurations"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let pip_id = lower(&cfg["properties"]["publicIPAddress"]["id"]);
                if !pip_id.is_empty() {
                    addr = ips["value"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|p| lower(&p["id"]) == pip_id)
                        .and_then(|p| p["properties"]["ipAddress"].as_str().map(|s| s.to_string()));
                }
                if addr.is_none() {
                    addr = cfg["properties"]["privateIPAddress"]
                        .as_str()
                        .map(|s| s.to_string());
                }
            }
        }
        if let Some(a) = addr {
            let mut h = host(name, a, "azure", user.as_deref());
            h.os = vm["properties"]["storageProfile"]["imageReference"]["offer"]
                .as_str()
                .map(|s| s.to_string());
            out.push(h);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from the AWS SigV4 documentation example key.
    #[test]
    fn sigv4_signature_is_stable_and_well_formed() {
        let (auth, payload) = sigv4_headers(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "ec2",
            "ec2.us-east-1.amazonaws.com",
            "Action=DescribeInstances&Version=2016-11-15",
            "20150830T123600Z",
        );
        assert_eq!(
            payload,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/ec2/aws4_request"
        ));
        let sig = auth.rsplit("Signature=").next().unwrap();
        assert_eq!(sig.len(), 64);
        let again = sigv4_headers(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "ec2",
            "ec2.us-east-1.amazonaws.com",
            "Action=DescribeInstances&Version=2016-11-15",
            "20150830T123600Z",
        );
        assert_eq!(auth, again.0);
    }
}

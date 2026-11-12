use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

pub const CA_NAME: &str = "WXGeo Local CA";

pub struct Ca {
    pub ca_pem: String,
    pub ca_key_pem: String,
}

pub fn ca_pem_path() -> PathBuf { crate::config::ca_dir().join("ca.pem") }

impl Ca {
    pub fn ensure() -> Result<Ca> {
        let dir = crate::config::ca_dir();
        fs::create_dir_all(&dir)?;
        let cert_path = dir.join("ca.pem");
        let key_path = dir.join("ca.key");
        if cert_path.exists() && key_path.exists() {
            return Ok(Ca {
                ca_pem: fs::read_to_string(&cert_path)?,
                ca_key_pem: fs::read_to_string(&key_path)?,
            });
        }
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
        let mut p = CertificateParams::default();
        p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        p.distinguished_name.push(DnType::CommonName, CA_NAME);
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let kp = KeyPair::generate()?;
        let cert = p.self_signed(&kp)?;
        let ca_pem = cert.pem();
        let ca_key_pem = kp.serialize_pem();
        fs::write(&cert_path, &ca_pem)?;
        fs::write(&key_path, &ca_key_pem)?;
        Ok(Ca { ca_pem, ca_key_pem })
    }

    pub fn leaf(&self, host: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        use rcgen::{
            CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
        };
        let ca_kp = KeyPair::from_pem(&self.ca_key_pem)?;
        let ca_params = CertificateParams::from_ca_cert_pem(&self.ca_pem)
            .map_err(|e| anyhow!("解析 CA 失败: {e}"))?;
        let ca_cert = ca_params.self_signed(&ca_kp)?;

        let mut p = CertificateParams::new(vec![host.to_string()])
            .map_err(|e| anyhow!("构造证书参数失败: {e}"))?;
        p.distinguished_name.push(DnType::CommonName, host);
        p.is_ca = IsCa::NoCa;
        p.use_authority_key_identifier_extension = true;
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf_kp = KeyPair::generate()?;
        let leaf = p.signed_by(&leaf_kp, &ca_cert, &ca_kp)?;
        Ok((leaf.der().to_vec(), leaf_kp.serialize_der()))
    }
}

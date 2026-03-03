use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use hex;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_sigv4(
    method: &str,
    uri: &str,
    query: &str,
    headers: &BTreeMap<String, String>,
    payload_hash: &str,
    access_key: &str,
    secret_key: &str,
    region: &str,
    service: &str,
    amz_date: &str, // Format: YYYYMMDDThhmmssZ
    date_stamp: &str, // Format: YYYYMMDD
) -> (String, String) {
    // 1. Create Canonical Request
    let mut canonical_headers = String::new();
    let mut signed_headers = String::new();

    // Headers must be sorted by key (lowercase)
    let mut sorted_headers: Vec<(&String, &String)> = headers.iter().collect();
    sorted_headers.sort_by_key(|(k, _)| k.to_lowercase());

    for (k, v) in sorted_headers {
        let k_lower = k.to_lowercase();
        let mut v_clean = v.trim().to_string();
        while v_clean.contains("  ") {
            v_clean = v_clean.replace("  ", " ");
        }
        canonical_headers.push_str(&format!("{}:{}\n", k_lower, v_clean));
        signed_headers.push_str(&format!("{};", k_lower));
    }
    signed_headers.pop(); // Remove trailing ';'

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, uri, query, canonical_headers, signed_headers, payload_hash
    );

    let canonical_request_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));

    // 2. Create String to Sign
    let algorithm = "AWS4-HMAC-SHA256";
    let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}",
        algorithm, amz_date, credential_scope, canonical_request_hash
    );

    // 3. Calculate Signature
    let k_secret = format!("AWS4{}", secret_key);
    let mut mac1 = HmacSha256::new_from_slice(k_secret.as_bytes()).unwrap();
    mac1.update(date_stamp.as_bytes());
    let k_date = mac1.finalize().into_bytes();

    let mut mac2 = HmacSha256::new_from_slice(&k_date).unwrap();
    mac2.update(region.as_bytes());
    let k_region = mac2.finalize().into_bytes();

    let mut mac3 = HmacSha256::new_from_slice(&k_region).unwrap();
    mac3.update(service.as_bytes());
    let k_service = mac3.finalize().into_bytes();

    let mut mac4 = HmacSha256::new_from_slice(&k_service).unwrap();
    mac4.update(b"aws4_request");
    let k_signing = mac4.finalize().into_bytes();

    let mut mac5 = HmacSha256::new_from_slice(&k_signing).unwrap();
    mac5.update(string_to_sign.as_bytes());
    let signature_bytes = mac5.finalize().into_bytes();

    let signature = hex::encode(signature_bytes);

    // 4. Create Authorization Header
    let authorization_header = format!(
        "{} Credential={}/{}, SignedHeaders={}, Signature={}",
        algorithm, access_key, credential_scope, signed_headers, signature
    );

    (authorization_header, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_sigv4_reference() {
        // Test vectors from official AWS documentation
        // https://docs.aws.amazon.com/AmazonS3/latest/API/sig-v4-header-based-auth.html

        let method = "GET";
        let uri = "/test.txt";
        let query = "";

        let mut headers = BTreeMap::new();
        headers.insert("Host".to_string(), "examplebucket.s3.amazonaws.com".to_string());
        headers.insert("x-amz-content-sha256".to_string(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string());
        headers.insert("x-amz-date".to_string(), "20130524T000000Z".to_string());
        headers.insert("Range".to_string(), "bytes=0-9".to_string());

        let payload_hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let access_key = "AKIAIOSFODNN7EXAMPLE";
        let secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let region = "us-east-1";
        let service = "s3";
        let amz_date = "20130524T000000Z";
        let date_stamp = "20130524";

        let (auth_header, signature) = generate_sigv4(
            method, uri, query, &headers, payload_hash, access_key, secret_key, region, service, amz_date, date_stamp
        );

        assert_eq!(signature, "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");

        let expected_auth = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41";
        assert_eq!(auth_header, expected_auth);
    }
}

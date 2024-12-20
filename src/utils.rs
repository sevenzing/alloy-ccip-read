use anyhow::bail;
use std::{iter::successors, time::Duration};

/// Encodes a domain name into its binary representation according to the DNS
/// wire format. Each label (i.e., substring separated by dots) in the domain
/// is prefixed with its length, and the encoded domain name is terminated
/// with a root label (length 0).
///
/// # Arguments
///
/// * `domain` - A domain name as a string (e.g., "tanrikulu.eth").
///
/// # Returns
///
/// * A `Result` containing the encoded domain name as a `Vec<u8>` on success, or an error message
///   as a `String` if any of the labels in the domain name are too long (exceeding 63 characters).
///
/// # Example
///
/// ```
/// use alloy_ccip_read::utils::dns_encode;
///
/// let encoded = dns_encode("tanrikulu.eth").unwrap();
/// assert_eq!(encoded, vec![9, b't', b'a', b'n', b'r', b'i', b'k', b'u', b'l', b'u', 3, b'e', b't', b'h', 0]);
/// ```
pub fn dns_encode(domain: &str) -> Result<Vec<u8>, anyhow::Error> {
    let mut encoded = Vec::new();
    let labels = domain.split('.');

    for label in labels {
        let label_len = label.len();
        if label_len > 63 {
            bail!("Label is too long: {}", label)
        }

        encoded.push(label_len as u8);
        encoded.extend(label.as_bytes());
    }

    // Append the root label (length 0)
    encoded.push(0);

    Ok(encoded)
}

/// Returns an iterator over the parent names of a given domain name.
/// ```
/// use alloy_ccip_read::utils::iter_parent_names;
///
/// let parent_names = iter_parent_names("tanrikulu.eth");
/// assert_eq!(parent_names, vec!["tanrikulu.eth", "eth"]);
/// ```
pub fn iter_parent_names(name: &str) -> Vec<&str> {
    successors(Some(name), |&last| last.split_once('.').map(|it| it.1)).collect()
}

pub(crate) fn truncate_str(src: &str, side: usize) -> String {
    if src.len() < side * 2 + 3 {
        return src.to_string();
    }

    format!("{}..{}", &src[..side], &src[src.len() - side..])
}

pub(crate) fn build_reqwest(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .expect("should be a valid reqwest client")
}

pub(crate) fn sanitaze_error_data_from_rpc(data: String) -> String {
    data.trim_start_matches("Reverted").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dns_encode() {
        let ens_name = "vitalik.eth";
        let expected_result = vec![
            7, b'v', b'i', b't', b'a', b'l', b'i', b'k', 3, b'e', b't', b'h', 0,
        ];

        let encoded_name = dns_encode(ens_name).unwrap();

        assert_eq!(
            encoded_name, expected_result,
            "Expected dns encoded result to be {:?}, but got {:?}",
            expected_result, encoded_name
        );
    }

    #[test]
    fn test_dns_encode_with_a_long_name() {
        let ens_name = "superlongbutmeaningfulnameforanextraordinaryrustproject1234567890x.eth";
        let mut labels = ens_name.split('.');

        let encoded_name = dns_encode(ens_name);

        assert_eq!(
            encoded_name.unwrap_err().to_string(),
            format!("Label is too long: {}", labels.next().unwrap())
        );
    }
}

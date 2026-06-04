use crate::{
    errors::{CCIPFetchError, CCIPReaderError, CCIPRequestError},
    utils::truncate_str,
    CCIPRequest, CCIPResponse,
};
use alloy::{
    hex,
    primitives::{Address, Bytes},
    rpc::types::TransactionRequest,
};
use reqwest::Response;
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

/// This function makes a Cross-Chain Interoperability Protocol (CCIP-Read) request
/// and returns the result as `Bytes` or an error message.
///
/// # Arguments
///
/// * `sender`: The sender's address.
/// * `tx`: The typed transaction.
/// * `calldata`: The function call data as bytes.
/// * `urls`: A vector of Offchain Gateway URLs to send the request to.
///
/// # Returns
///
/// an opaque byte string to send to callbackFunction on Offchain Resolver contract.
pub async fn handle_ccip(
    client: &reqwest::Client,
    sender: &Address,
    tx: &TransactionRequest,
    calldata: &[u8],
    urls: Vec<String>,
) -> Result<(Bytes, Vec<CCIPRequest>), CCIPReaderError> {
    // If there are no URLs or the transaction's destination is empty, return an empty result
    if urls.is_empty() || tx.to.is_none() {
        return Ok((Bytes::new(), Vec::new()));
    }

    let urls = dedup_ord(&urls);

    // url —> [error_message]
    let mut errors: HashMap<String, Vec<String>> = HashMap::new();

    let mut requests = Vec::new();

    for url in urls {
        let result = handle_ccip_raw(client, &url, sender, calldata).await;
        requests.push(CCIPRequest {
            url: url.clone(),
            sender: *sender,
            calldata: calldata.to_vec().into(),
        });

        match result {
            Ok(result) => return Ok((result, requests)),
            Err(err) => {
                errors.entry(url).or_default().push(err.to_string());
            }
        }
    }

    Err(CCIPReaderError::Fetch(CCIPFetchError(errors)))
}

const SENDER_PLACEHOLDER: &str = "{sender}";
const DATA_PLACEHOLDER: &str = "{data}";

pub async fn handle_ccip_raw(
    client: &reqwest::Client,
    url: &str,
    sender: &Address,
    calldata: &[u8],
) -> Result<Bytes, CCIPRequestError> {
    tracing::debug!("making CCIP request to {url}");

    let sender_hex = hex::encode_prefixed(sender.0);
    let data_hex: String = hex::encode_prefixed(calldata);

    tracing::debug!("sender: {}", sender_hex);
    tracing::debug!("data: {}", truncate_str(&data_hex, 20));

    let request = if url.contains(DATA_PLACEHOLDER) {
        let href = url
            .replace(SENDER_PLACEHOLDER, &sender_hex)
            .replace(DATA_PLACEHOLDER, &data_hex);

        client.get(href)
    } else {
        let body = serde_json::json!({
            "data": data_hex,
            "sender": sender_hex
        });

        client.post(url).json(&body)
    };

    let resp: Response = request.send().await?;

    let resp_text = resp.text().await?;

    // TODO: handle non-json responses
    //  in case of erroneous responses, server can return Content-Type that is not application/json
    //  in this case, we should read the response as text and perhaps treat that as the error
    let result: CCIPResponse = serde_json::from_str(&resp_text).map_err(|err| {
        CCIPRequestError::GatewayFormatError(format!(
            "response format error: {err}, gateway returned: {resp_text}"
        ))
    })?;

    if let Some(response_data) = result.data {
        return hex::decode(response_data)
            .map_err(|_| {
                CCIPRequestError::GatewayFormatError(
                    "response data is not a valid hex sequence".to_string(),
                )
            })
            .map(Bytes::from);
    };

    if let Some(message) = result.message {
        return Err(CCIPRequestError::GatewayError(message));
    }

    Err(CCIPRequestError::GatewayFormatError(
        "response format error: invalid response".to_string(),
    ))
}

fn dedup_ord<T: Clone + Hash + Eq>(src: &[T]) -> Vec<T> {
    let mut set = HashSet::new();

    let mut copy = src.to_vec();
    copy.retain(|item| set.insert(item.clone()));

    copy
}

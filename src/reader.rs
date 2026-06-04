use crate::{
    ccip, consts, contracts,
    domain_id::DomainIdProvider,
    errors::CCIPReaderError,
    utils::{build_reqwest, sanitaze_error_data_from_rpc},
    CCIPRequest, CCIPResult, NamehashIdProvider,
};
use alloy::{
    eips::BlockId,
    hex::FromHex,
    network::TransactionBuilder,
    primitives::{Address, Bytes},
    providers::{ProviderBuilder, RootProvider},
    rpc::types::TransactionRequest,
    sol_types::{SolCall, SolError, SolValue},
    transports::{RpcError, TransportErrorKind},
};
use async_recursion::async_recursion;
use reqwest::Url;
use serde_json::Value;
use std::time::Duration;

pub struct CCIPReader<P, D> {
    provider: P,
    reqwest_client: reqwest::Client,
    max_redirect_attempt: u8,
    domain_id_provider: D,
}

pub struct CCIPReaderBuilder<P, D> {
    provider: Option<P>,
    timeout: Option<Duration>,
    max_redirect_attempt: Option<u8>,
    domain_id_provider: D,
}

impl<P> Default for CCIPReaderBuilder<P, NamehashIdProvider> {
    fn default() -> Self {
        CCIPReaderBuilder {
            provider: None,
            timeout: None,
            max_redirect_attempt: None,
            domain_id_provider: NamehashIdProvider,
        }
    }
}

impl<P, D> CCIPReaderBuilder<P, D>
where
    P: alloy::providers::Provider + Clone,
    D: DomainIdProvider,
{
    pub fn with_provider(mut self, provider: P) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_max_redirect_attempt(mut self, max_redirect_attempt: u8) -> Self {
        self.max_redirect_attempt = Some(max_redirect_attempt);
        self
    }

    pub fn with_domain_id_provider<D2>(self, domain_id_provider: D2) -> CCIPReaderBuilder<P, D2> {
        CCIPReaderBuilder {
            provider: self.provider,
            timeout: self.timeout,
            max_redirect_attempt: self.max_redirect_attempt,
            domain_id_provider,
        }
    }

    pub fn build(self) -> Result<CCIPReader<P, D>, String> {
        let provider = self.provider.ok_or("provider is required".to_string())?;
        Ok(CCIPReader {
            provider,
            reqwest_client: build_reqwest(self.timeout.unwrap_or(Duration::from_secs(10))),
            max_redirect_attempt: self.max_redirect_attempt.unwrap_or(10),
            domain_id_provider: self.domain_id_provider,
        })
    }
}

impl<P, D> CCIPReader<P, D> {
    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn domain_id_provider(&self) -> &D {
        &self.domain_id_provider
    }
}

impl<P> CCIPReader<P, NamehashIdProvider>
where
    P: alloy::providers::Provider + Clone,
{
    pub fn builder() -> CCIPReaderBuilder<P, NamehashIdProvider> {
        CCIPReaderBuilder::default()
    }

    /// Creates an instance of CCIPReader
    /// `ìnner` the inner Provider
    pub fn new(inner: P) -> Self {
        Self::builder().with_provider(inner).build().unwrap()
    }
}

impl CCIPReader<RootProvider, NamehashIdProvider> {
    pub fn on_http(url: impl Into<Url>) -> Self {
        let provider: alloy::providers::RootProvider =
            ProviderBuilder::default().connect_http(url.into());
        Self::builder().with_provider(provider).build().unwrap()
    }

    pub fn mainnet() -> Self {
        Self::on_http(consts::DEFAULT_ETHEREUM_RPC_URL.parse::<Url>().unwrap())
    }
}

impl<P, D> CCIPReader<P, D>
where
    P: alloy::providers::Provider,
    D: DomainIdProvider + Send + Sync,
{
    pub async fn call_and_only_result(
        &self,
        tx: &TransactionRequest,
    ) -> Result<Bytes, CCIPReaderError> {
        self.call_ccip(tx).await.map(|result| result.into_value())
    }

    /// Perform eth_call with tx, and handle CCIP requests if needed
    /// returning both the result of the call and the CCIP requests made during the call
    pub async fn call_ccip(
        &self,
        tx: &TransactionRequest,
    ) -> Result<CCIPResult<Bytes>, CCIPReaderError> {
        let mut requests = Vec::new();
        let mut tx = tx.clone();
        self._call(&mut tx, 0, &mut requests).await
    }

    /// Perform eth_call with tx, and handle CCIP requests if needed
    /// returning both the result of the call and the CCIP requests made during the call
    pub async fn call_ccip_solidity_method<C: SolCall>(
        &self,
        contract_address: Address,
        call: C,
    ) -> Result<CCIPResult<<C as SolCall>::Return>, CCIPReaderError> {
        let tx = TransactionRequest::default()
            .with_to(contract_address)
            .with_call(&call);
        let result = self.call_ccip(&tx).await?;
        let value = C::abi_decode_returns(&result.value)?;
        Ok(CCIPResult::new(value, result.requests))
    }

    #[tracing::instrument(
        skip(self, transaction, requests_buffer),
        fields(tx_to =? transaction.to),
        level = "debug"
    )]
    #[async_recursion]
    async fn _call(
        &self,
        transaction: &mut TransactionRequest,
        attempt: u8,
        requests_buffer: &mut Vec<CCIPRequest>,
    ) -> Result<CCIPResult<Bytes>, CCIPReaderError> {
        if attempt >= self.max_redirect_attempt {
            // may need more info
            return Err(CCIPReaderError::MaxRedirection);
        }
        let response = self
            .provider
            .call(transaction.clone())
            .block(BlockId::latest())
            .await;

        match response {
            Ok(result) => Ok(CCIPResult::new(result, requests_buffer.to_vec())),
            Err(err) => {
                tracing::debug!("rpc-error: {:?}", err);
                self._handle_rpc_error(err, transaction, attempt, requests_buffer)
                    .await
            }
        }
    }

    async fn _handle_rpc_error(
        &self,
        err: RpcError<TransportErrorKind>,
        transaction: &mut TransactionRequest,
        attempt: u8,
        requests_buffer: &mut Vec<CCIPRequest>,
    ) -> Result<CCIPResult<Bytes>, CCIPReaderError> {
        let tx_sender = transaction
            .to
            .as_ref()
            .and_then(|to| to.to())
            .expect("call tx must have a to");

        let Some(error_payload) = err.as_error_resp() else {
            return Err(err.into());
        };
        let Some(Ok(Value::String(data))) = error_payload.try_data_as::<serde_json::Value>() else {
            return Err(err.into());
        };
        let data = sanitaze_error_data_from_rpc(data);
        let bytes = Bytes::from_hex(&data)?;

        if !bytes.starts_with(&consts::OFFCHAIN_LOOKUP_SELECTOR) {
            return Err(err.into());
        }

        let offchain_lookup = contracts::IOffChain::OffchainLookup::abi_decode(&bytes)?;
        let sender = &offchain_lookup.sender;
        if !sender.eq(tx_sender) {
            return Err(CCIPReaderError::Sender {
                sender: format!("0x{:x}", sender),
            });
        }

        let (ccip_result, requests) = ccip::handle_ccip(
            &self.reqwest_client,
            &offchain_lookup.sender,
            transaction,
            &offchain_lookup.callData,
            offchain_lookup.urls,
        )
        .await?;

        requests_buffer.extend(requests);

        if ccip_result.is_empty() {
            return Err(CCIPReaderError::GatewayNotFound);
        }

        let encoded_data = (ccip_result, offchain_lookup.extraData).abi_encode_params();
        let calldata = [
            offchain_lookup.callbackFunction.as_slice(),
            encoded_data.as_slice(),
        ]
        .concat();

        transaction.set_input(calldata);

        self._call(transaction, attempt + 1, requests_buffer).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        hex,
        primitives::address,
        providers::{ProviderBuilder, RootProvider},
    };
    use pretty_assertions::assert_eq;

    fn default_provider() -> RootProvider {
        ProviderBuilder::default().connect_http(consts::DEFAULT_ETHEREUM_RPC_URL.parse().unwrap())
    }

    fn default_reader() -> CCIPReader<RootProvider, NamehashIdProvider> {
        CCIPReader::new(default_provider())
    }

    #[tokio::test]
    async fn test_ccip_call() {
        let reader = default_reader();

        let email = "nick@ens.domains";

        // parameters = text(bytes32 node, string calldata key) node: namehash('1.offchainexample.eth'), key: 'email'
        // tx_data = selector(resolve(bytes,bytes)), namehash(name), parameters
        // ensip10 interface + encode(dnsencode(name), tx_data)
        let tx = TransactionRequest::default()
            .with_input(hex!("9061b92300000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000001701310f6f6666636861696e6578616d706c650365746800000000000000000000000000000000000000000000000000000000000000000000000000000000008459d1d43c1c9fb8c1fe76f464ccec6d2c003169598fdfcbcb6bbddf6af9c097a39fa0048c00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000005656d61696c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"))
            .with_to(address!("C1735677a60884ABbCF72295E88d47764BeDa282"));

        let result = reader.call_and_only_result(&tx).await.unwrap();

        let data: Bytes = Bytes::abi_decode(&result).unwrap();
        let record: String = String::abi_decode(&data).unwrap();

        assert_eq!(record, email);
    }

    // // TODO: rewrite this test when alloy will support mocked provider
    // #[tokio::test]
    // async fn test_mismatched_sender() {
    //     let resolver_address = "0xC1735677a60884ABbCF72295E88d47764BeDa282";

    //     let (provider, mock) = Provider::mocked();
    //     let provider = CCIPReader::new(provider);

    //     let tx: TypedTransaction = TransactionRequest {
    //         // parameters = text(bytes32 node, string calldata key) node: namehash('1.offchainexample.eth'), key: 'email'
    //         // tx_data = selector(resolve(bytes,bytes)), namehash(name), parameters
    //         // ensip10 interface + encode(dnsencode(name), tx_data)
    //         data: Some(Bytes::from(hex::decode("9061b92300000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000001701310f6f6666636861696e6578616d706c650365746800000000000000000000000000000000000000000000000000000000000000000000000000000000008459d1d43c1c9fb8c1fe76f464ccec6d2c003169598fdfcbcb6bbddf6af9c097a39fa0048c00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000005656d61696c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap())),
    //         to: Some(resolver_address.into()),
    //         ..Default::default()
    //     }.into();

    //     let error_code = 3;
    //     // sender information altered to c1735677a60884abbcf72295e88d47764beda283
    //     let error_data = r#""0x556f1830000000000000000000000000c1735677a60884abbcf72295e88d47764beda28300000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000160f4d4d2f80000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000004768747470733a2f2f6f6666636861696e2d7265736f6c7665722d6578616d706c652e75632e722e61707073706f742e636f6d2f7b73656e6465727d2f7b646174617d2e6a736f6e0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001449061b92300000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000001701310f6f6666636861696e6578616d706c650365746800000000000000000000000000000000000000000000000000000000000000000000000000000000008459d1d43c1c9fb8c1fe76f464ccec6d2c003169598fdfcbcb6bbddf6af9c097a39fa0048c00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000005656d61696c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001449061b92300000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080000000000000000000000000000000000000000000000000000000000000001701310f6f6666636861696e6578616d706c650365746800000000000000000000000000000000000000000000000000000000000000000000000000000000008459d1d43c1c9fb8c1fe76f464ccec6d2c003169598fdfcbcb6bbddf6af9c097a39fa0048c00000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000005656d61696c0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000""#;
    //     let error_message = "execution reverted";
    //     let error = JsonRpcError {
    //         code: error_code,
    //         data: Some(serde_json::from_str(error_data).unwrap()),
    //         message: error_message.to_string(),
    //     };
    //     mock.push_response(MockResponse::Error(error.clone()));

    //     let result = provider.call(&tx, None).await;
    //     assert!(result.is_err());
    //     assert_eq!(
    //         result.unwrap_err().to_string(),
    //         format!(
    //             "CCIP Read sender did not match {}",
    //             "0xc1735677a60884abbcf72295e88d47764beda283"
    //         )
    //     );
    // }
}

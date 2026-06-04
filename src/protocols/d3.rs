use super::ens;
use crate::{
    contracts, CCIPReader, CCIPReaderError, DomainIdProvider, ResolveResult, ReverseResolveResult,
};
use alloy::{primitives::Address, providers::Provider};

/// Resolves a D3 name.
///
/// # Arguments
///
/// * `reader`: The CCIP reader.
/// * `resolver_address`: The resolver's address.
/// * `name`: The name to resolve.
/// * `network`: The network to resolve the name on. Empty string means current network where resolver deployed.
pub async fn resolve_d3_name<P: Provider, D: DomainIdProvider>(
    reader: &CCIPReader<P, D>,
    resolver_address: Address,
    name: impl Into<String>,
    network: impl Into<String>,
) -> Result<ResolveResult, CCIPReaderError> {
    let call = contracts::D3Connect::resolveCall {
        name: name.into(),
        network: network.into(),
    };
    let mut ccip_read_used = false;
    let addr = ens::query_resolver_non_wildcarded(reader, resolver_address, call).await?;
    ccip_read_used |= addr.ccip_read_used();
    Ok(ResolveResult {
        addr,
        ccip_read_used,
        wildcard_used: false,
    })
}

/// Reverse resolves a D3 name.
///
/// # Arguments
///
/// * `reader`: The CCIP reader.
/// * `address`: The address to reverse resolve.
/// * `resolver_address`: The resolver's address.
/// * `network`: The network to reverse resolve the name on. Empty string means current network where resolver deployed.
pub async fn reverse_resolve_d3_name<P: Provider, D: DomainIdProvider>(
    reader: &CCIPReader<P, D>,
    address: Address,
    resolver_address: Address,
    network: impl Into<String>,
) -> Result<ReverseResolveResult, CCIPReaderError> {
    let call = contracts::D3Connect::reverseResolveCall {
        addr: address,
        network: network.into(),
    };
    let mut ccip_read_used = false;
    let name = ens::query_resolver_non_wildcarded(reader, resolver_address, call).await?;
    ccip_read_used |= name.ccip_read_used();
    Ok(ReverseResolveResult {
        name,
        ccip_read_used,
    })
}

#[cfg(test)]
mod tests {
    use crate::{CCIPReader, NamehashIdProvider};
    use alloy::providers::{ProviderBuilder, RootProvider};
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    #[once]
    fn reader() -> CCIPReader<RootProvider, NamehashIdProvider> {
        let shibarium = ProviderBuilder::default()
            .connect_http("https://puppynet.shibrpc.com".parse().unwrap());
        CCIPReader::new(shibarium)
    }

    #[rstest]
    #[tokio::test]
    #[ignore = "D3 http server is down"]
    #[case(
        "0x91c2d22ca1028B2E55e3097096494Eb34b7fc81c",
        "d3connect.shib",
        "",
        "0x7309E26de18FD96A34aBBE6063a0b08d78c947C5"
    )]
    async fn test_shibarium_d3connect(
        #[case] resolver_address: Address,
        #[case] name: &str,
        #[case] network: &str,
        #[case] address: Address,
        reader: &CCIPReader<RootProvider, NamehashIdProvider>,
    ) {
        let result = resolve_d3_name(reader, resolver_address, name, network)
            .await
            .unwrap();
        assert_eq!(
            result.addr.value,
            "0x7309E26de18FD96A34aBBE6063a0b08d78c947C5"
                .parse::<Address>()
                .unwrap()
        );

        let result = reverse_resolve_d3_name(reader, address, resolver_address, network)
            .await
            .unwrap();
        assert_eq!(result.name.value, "d3connect.shib");
    }
}

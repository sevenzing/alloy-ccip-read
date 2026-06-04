use crate::{
    consts, contracts,
    utils::{dns_encode, iter_parent_names},
    CCIPReader, CCIPReaderError, CCIPResult, DomainIdProvider, ResolveResult,
};
use alloy::{
    primitives::{Address, Bytes},
    providers::Provider,
    sol_types::{SolCall, SolValue},
};

/// The supports_wildcard checks if a given resolver supports the wildcard resolution by calling
/// its `supportsInterface` function with the `resolve(bytes,bytes)` selector.
///
/// # Arguments
///
/// * `resolver_address`: The resolver's address.
///
/// # Returns
///
/// A `Result` with either a `bool` value indicating if the resolver supports wildcard
/// resolution or a `ProviderError`.
pub async fn supports_wildcard<P: Provider, D>(
    reader: &CCIPReader<P, D>,
    resolver_address: Address,
) -> Result<bool, CCIPReaderError> {
    let contract = contracts::IERC165::new(resolver_address, reader.provider());
    // sending `supportsInterface` call, with selector of the "resolve(bytes,bytes)" function
    let supported = contract
        .supportsInterface(consts::ENSIP10_RESOLVER_INTERFACE.into())
        .call()
        .await?;
    Ok(supported)
}

/// Get the resolver for a given name using wildcard resolution
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `registry_address` - Optional address of the ENS registry contract, if None, the default registry address will be used
/// * `name` - The ENS name to resolve
///
pub async fn get_resolver_wildcarded<P: Provider, D: DomainIdProvider>(
    reader: &CCIPReader<P, D>,
    registry_address: Option<Address>,
    name: &str,
) -> Result<Address, CCIPReaderError> {
    let registry = registry(registry_address, reader.provider());
    for parent_name in iter_parent_names(name) {
        if parent_name.is_empty() || parent_name.eq(".") {
            return Ok(Address::ZERO);
        }

        if !name.eq("eth") && parent_name.eq("eth") {
            return Ok(Address::ZERO);
        }

        let name_id = reader.domain_id_provider().generate(parent_name);
        let data = registry.resolver(name_id).call().await?;
        let resolver_address = data;

        if resolver_address != Address::ZERO {
            if parent_name != name && !supports_wildcard(reader, resolver_address).await? {
                return Ok(Address::ZERO);
            }
            return Ok(resolver_address);
        }
    }

    Ok(Address::ZERO)
}

/// Resolves a name to an address.
/// This function queries the registry for the resolver contract address associated with the given name,
/// and then uses that resolver to obtain the corresponding address.
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `registry_address` - Optional address of the ENS registry contract, if None, the default registry address will be used
/// * `name` - The ENS name to resolve
///
/// # Returns
///
/// Returns a `Result` containing a `ResolveResult` with the resolved address and additional information,
/// or a `CCIPReaderError` if the resolution fails.
///
pub async fn resolve_name<P: Provider, D: DomainIdProvider>(
    reader: &CCIPReader<P, D>,
    registry_address: Option<Address>,
    name: &str,
) -> Result<ResolveResult, CCIPReaderError> {
    let resolver_address = get_resolver_wildcarded(reader, registry_address, name).await?;
    resolve_name_with_resolver(reader, name, resolver_address).await
}

/// Resolve a name to an address with a given resolver
/// This function uses the provided resolver to resolve the name to an address.
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `name` - The ENS name to resolve
/// * `resolver_address` - The address of the resolver to use
///
pub async fn resolve_name_with_resolver<P: Provider, D: DomainIdProvider>(
    reader: &CCIPReader<P, D>,
    name: &str,
    resolver_address: Address,
) -> Result<ResolveResult, CCIPReaderError> {
    let supports_wildcard = supports_wildcard(reader, resolver_address).await?;
    let node = reader.domain_id_provider().generate(name);
    let addr_call = contracts::IAddrResolver::addrCall { node };
    let mut ccip_read_used = false;
    let addr = query_resolver(reader, name, resolver_address, addr_call, supports_wildcard).await?;
    ccip_read_used |= !addr.requests.is_empty();
    Ok(ResolveResult {
        addr,
        ccip_read_used,
        wildcard_used: supports_wildcard,
    })
}

/// Query a resolver for a given ENS name and call
///
/// This function determines whether to use a wildcarded or non-wildcarded query based on the `supports_wildcard` parameter.
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `name` - The ENS name to resolve
/// * `resolver_address` - The address of the resolver to query
/// * `call` - The SolCall to execute on the resolver
/// * `supports_wildcard` - Whether the resolver supports wildcard queries
///
/// # Returns
///
/// Returns a `Result` containing a tuple of the call's return value and a vector of CCIPRequests,
/// or a `CCIPReaderError` if the query fails.
pub async fn query_resolver<P: Provider, D: DomainIdProvider, C: SolCall>(
    reader: &CCIPReader<P, D>,
    name: &str,
    resolver_address: Address,
    call: C,
    supports_wildcard: bool,
) -> Result<CCIPResult<<C as SolCall>::Return>, CCIPReaderError> {
    if supports_wildcard {
        query_resolver_wildcarded(reader, name, resolver_address, call).await
    } else {
        query_resolver_non_wildcarded(reader, resolver_address, call).await
    }
}

/// Query a resolver for a given ENS name and call using wildcard resolution
/// This function performs a wildcard query on the resolver for the specified ENS name and call.
/// It encodes the name using DNS encoding and prepares the call data for the resolver.
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `name` - The ENS name to resolve
/// * `resolver_address` - The address of the resolver to query
/// * `call` - The SolCall to execute on the resolver
///
/// # Returns
///
/// Returns a `Result` containing a tuple of the call's return value and a vector of CCIPRequests,
/// or a `CCIPReaderError` if the query fails.
pub async fn query_resolver_wildcarded<P: Provider, D: DomainIdProvider, C: SolCall>(
    reader: &CCIPReader<P, D>,
    name: &str,
    resolver_address: Address,
    call: C,
) -> Result<CCIPResult<<C as SolCall>::Return>, CCIPReaderError> {
    let dns_encode_name =
        dns_encode(name).map_err(|e| CCIPReaderError::InvalidDomain(e.to_string()))?;
    let data = call.abi_encode();
    let resolver = contracts::IExtendedResolver::new(resolver_address, reader.provider());
    let tx = resolver
        .resolve(dns_encode_name.into(), data.into())
        .into_transaction_request();

    let result = reader.call_ccip(&tx).await?;

    tracing::debug!(requests =? result.requests, "finished call_ccip");

    let bytes = Bytes::abi_decode(&result.value)?;

    let value = C::abi_decode_returns(&bytes)?;

    Ok(CCIPResult::new(value, result.requests))
}

/// Query a resolver for a given ENS name and call without using wildcard resolution
///
/// This function performs a non-wildcard query on the resolver for the specified ENS name and call.
/// It prepares the call data for the resolver and sends it directly to the resolver.
///
/// # Arguments
///
/// * `reader` - The CCIPReader instance used for making queries
/// * `resolver_address` - The address of the resolver to query
/// * `call` - The SolCall to execute on the resolver
///
pub async fn query_resolver_non_wildcarded<P: Provider, D: DomainIdProvider, C: SolCall>(
    reader: &CCIPReader<P, D>,
    resolver_address: Address,
    call: C,
) -> Result<CCIPResult<<C as SolCall>::Return>, CCIPReaderError> {
    reader
        .call_ccip_solidity_method(resolver_address, call)
        .await
}

fn registry<P: Provider>(address: Option<Address>, provider: P) -> contracts::ENS::ENSInstance<P> {
    contracts::ENS::new(address.unwrap_or(consts::MAINNET_ENS_ADDRESS), provider)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{namehash, DomainIdProvider};
    use alloy::{primitives::B256, providers::ProviderBuilder};
    use pretty_assertions::assert_eq;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_custom_domain_id_provider() {
        let reader_always_appends_eth = {
            #[derive(Clone)]
            struct CustomDomainIdProvider;
            impl DomainIdProvider for CustomDomainIdProvider {
                fn generate(&self, name: &str) -> B256 {
                    namehash(&format!("{}.eth", name))
                }
            }
            let provider = ProviderBuilder::default()
                .connect_http(consts::DEFAULT_ETHEREUM_RPC_URL.parse().unwrap());
            CCIPReader::builder()
                .with_provider(provider)
                .with_domain_id_provider(CustomDomainIdProvider)
                .build()
                .unwrap()
        };

        let resolver_address = resolve_name(&reader_always_appends_eth, None, "vitalik")
            .await
            .unwrap()
            .addr
            .value;
        assert_eq!(
            resolver_address,
            Address::from_str("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045").unwrap()
        );
    }

    use rstest::rstest;

    #[rstest]
    #[case(
        "1.offchainexample.eth",
        true,
        true,
        "0xC1735677a60884ABbCF72295E88d47764BeDa282",
        "0x41563129cDbbD0c5D3e1c86cf9563926b243834d"
    )]
    #[case(
        "llev.me",
        false,
        false,
        "0x231b0ee14048e9dccd1d247744d114a4eb5e8e63",
        "0xc0de20a37e2dac848f81a93bd85fe4acdde7c0de"
    )]
    #[case(
        "vitalik.eth",
        false,
        false,
        "0x231b0Ee14048e9dCcD1d247744d114a4EB5E8E63",
        "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
    )]
    #[case(
        "itslev.cb.id",
        true,
        true,
        "0x1934FC75aD10d7eEd51dc7A92773cAc96A06BE56",
        "0xD578780f1dA7404d9CC0eEbC9D684c140CC4b638"
    )]
    #[case(
        "moo331.nft-owner.eth",
        true,
        false,
        "0x56942dd93A6778F4331994A1e5b2f59613DE1387",
        "0x51050ec063d393217B436747617aD1C2285Aeeee"
    )]
    #[case(
        "offchaindemo.eth",
        true,
        true,
        "0x35b920d4329c5797727af8b15358b43509e5e237",
        "0x179A862703a4adfb29896552DF9e307980D19285"
    )]
    async fn test_ens_wildcards(
        #[case] ens_name: &str,
        #[case] expected_wildcarded: bool,
        #[case] expected_ccip_read_used: bool,
        #[case] expected_resolver: &str,
        #[case] expected_addr: &str,
    ) {
        let reader = CCIPReader::mainnet();

        let resolver_address = get_resolver_wildcarded(&reader, None, ens_name)
            .await
            .unwrap();
        assert_eq!(
            resolver_address,
            Address::from_str(expected_resolver).unwrap(),
            "{ens_name}: expected resolver_address to be {expected_resolver}, but got {}",
            resolver_address
        );

        let result = resolve_name(&reader, None, ens_name).await.unwrap();
        assert_eq!(
            result.addr.value,
            Address::from_str(expected_addr).unwrap(),
            "{ens_name}: expected resolved_address to be {expected_addr}, but got {}",
            result.addr.value
        );
        assert_eq!(
            result.ccip_read_used, expected_ccip_read_used,
            "{ens_name}: expected ccip_read_used to be {expected_ccip_read_used}, but got {}",
            result.ccip_read_used
        );
        assert_eq!(
            result.wildcard_used, expected_wildcarded,
            "{ens_name}: wildcard_used is {}, expected to be {expected_wildcarded}",
            result.wildcard_used
        );
    }
}

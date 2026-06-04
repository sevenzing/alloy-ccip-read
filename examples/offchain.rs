use alloy::providers::ProviderBuilder;
use alloy_ccip_read::CCIPReader;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let rpc = alloy_ccip_read::consts::DEFAULT_ETHEREUM_RPC_URL;
    let provider = ProviderBuilder::new().connect_http(rpc.parse().unwrap());
    let reader = CCIPReader::new(provider);
    let registry_address = None;
    for ens_name in ["1.offchainexample.eth", "itslev.cb.id", "llev.me"] {
        println!("\nresolving name: {}", ens_name);

        let resolver_address =
            alloy_ccip_read::ens::get_resolver_wildcarded(&reader, registry_address, ens_name)
                .await
                .unwrap();
        println!("resolver_address: {:?}", resolver_address);

        let supports_wildcard = alloy_ccip_read::ens::supports_wildcard(&reader, resolver_address)
            .await
            .unwrap();
        println!("supports_wildcard: {:?}", supports_wildcard);

        let resolved_address =
            alloy_ccip_read::ens::resolve_name(&reader, registry_address, ens_name)
                .await
                .unwrap();
        println!("resolved_address: {:?}", resolved_address);
    }

    Ok(())
}

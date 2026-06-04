# Alloy-rs CCIP Reader

> NOTE: This is fork of [ethers-ccip-read](https://github.com/ensdomains/ethers-ccip-read) implemented for [alloy-rs](https://alloy.rs) crate

<!-- Badges -->
[![CI Status][ci-badge]][ci-url]
[![Crates.io][crates-badge]][crates-url]
[![Docs.rs][docs-badge]][docs-url]

<!-- Badge Images -->
[ci-badge]: https://github.com/sevenzing/alloy-ccip-read/actions/workflows/ci.yml/badge.svg
[ci-url]: https://github.com/sevenzing/alloy-ccip-read/actions/workflows/ci.yml
[crates-badge]: https://img.shields.io/crates/v/alloy-ccip-read.svg

<!-- Target URLs -->
[crates-url]: https://crates.io/crates/alloy-ccip-read
[docs-badge]: https://docs.rs/alloy-ccip-read/badge.svg
[docs-url]: https://docs.rs/alloy-ccip-read



## Install

```bash
cargo add alloy-ccip-read
```


or alternatively add it to your `Cargo.toml` file:

```toml
alloy-ccip-read = "0.1.0"
```

## Usage

```rust
use alloy::providers::ProviderBuilder;
use alloy_ccip_read::CCIPReader;

#[tokio::main]
async fn main() -> Result<()> {
    let rpc = "https://your.web3.provider";
    let provider = ProviderBuilder::new().connect_http(rpc.parse().unwrap());
    let reader = CCIPReader::new(provider);

    let ens_name = "1.offchainexample.eth";
    
    let resolver_address = reader.get_resolver(ens_name).await.unwrap();
    println!("resolver_address: {:?}", resolver_address);

    let supports_wildcard = reader.supports_wildcard(resolver_address).await.unwrap();
    println!("supports_wildcard: {:?}", supports_wildcard);

    let resolved_address = reader.resolve_name(ens_name).await.unwrap();
    println!("resolved_address: {:?}", resolved_address);
    
    Ok(())
}
```

For more examples, check out [the examples](./examples) directory


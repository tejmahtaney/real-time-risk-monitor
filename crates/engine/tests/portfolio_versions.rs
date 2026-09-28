//! `GetSpecs` and `LoadPortfolio` round trip through a spawned engine. Neither command
//! touches disk itself (see `crate::recorder`'s doc comment) — saving and loading a
//! version file is the caller's job, exercised end to end in `crates/server`'s own
//! tests instead.

use engine::{EngineCommand, EngineReply};
use tokio_util::sync::CancellationToken;

mod support;

#[tokio::test(start_paused = true)]
async fn get_specs_then_load_restores_the_original_book_exactly() {
    let engine = support::build_engine();
    let (_dir, storage) = support::temp_storage();
    let (handle, _join) = engine.spawn(
        std::time::Duration::from_millis(50),
        storage,
        CancellationToken::new(),
    );

    let EngineReply::Specs(original) = handle.send(EngineCommand::GetSpecs).await.unwrap().unwrap()
    else {
        unreachable!("expected Specs");
    };
    assert_eq!(original.len(), 3, "the fixture book has three positions");

    let new_spec = engine::PositionSpec {
        id: engine::PositionId::new("aapl-stock").unwrap(),
        underlying: engine::Symbol::new("AAPL").unwrap(),
        kind: engine::InstrumentKind::Equity,
        strike: None,
        expiry: None,
        quantity: 5.0,
        entry_price: None,
        iv: None,
    };
    handle
        .send(EngineCommand::AddPosition(new_spec))
        .await
        .unwrap()
        .unwrap();

    let EngineReply::Specs(changed) = handle.send(EngineCommand::GetSpecs).await.unwrap().unwrap()
    else {
        unreachable!("expected Specs");
    };
    assert_eq!(changed.len(), 4, "the added position should be active");

    let reply = handle
        .send(EngineCommand::LoadPortfolio {
            specs: original.clone(),
            source: "before-add.csv".to_owned(),
        })
        .await
        .unwrap()
        .unwrap();
    let EngineReply::Loaded(count) = reply else {
        unreachable!("expected Loaded, got {reply:?}");
    };
    assert_eq!(count, original.len());

    let EngineReply::Specs(restored) = handle.send(EngineCommand::GetSpecs).await.unwrap().unwrap()
    else {
        unreachable!("expected Specs");
    };
    assert_eq!(
        restored, original,
        "the reloaded book should match the original exactly: ids, quantities, entry prices"
    );
}

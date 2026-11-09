//! A catalog for tests: what the binary ships, an event `:e` that hands a script its payload, a
//! `github.pull` that answers with what it was asked, and an `ages.age` that counts its calls on
//! the run's [`Ages`] provider.

use crate::Catalog;
use crate::catalog::{Read, Reading};
use almanac::cx::{Cx, StepError};
use almanac::taint::Untrusted;
use futures::future::BoxFuture;
use serde_json::Value as Data;
use std::sync::atomic::{AtomicU64, Ordering};

almanac::observed! {
    pub struct Echo {
        pub args: Untrusted<Vec<Data>>,
    }
}

fn echo(_: &Cx, args: Vec<Data>) -> BoxFuture<'_, Result<Echo, StepError>> {
    Box::pin(async move {
        Ok(Echo {
            args: Untrusted::new(args),
        })
    })
}

/// A provider that answers how many times it has been asked.
#[derive(Default)]
pub struct Ages(pub AtomicU64);

almanac::observed! {
    pub struct Age {
        pub calls: u64,
    }
}

fn age(cx: &Cx, _: Vec<Data>) -> BoxFuture<'_, Result<Age, StepError>> {
    Box::pin(async move {
        let ages = cx.get::<Ages>()?;
        let calls = ages.0.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Age { calls })
    })
}

pub fn catalog() -> Catalog {
    let mut catalog = Catalog::shipped();
    catalog
        .events
        .push(crate::catalog::Event::new("e", |payload| {
            Ok(Reading {
                data: payload.clone(),
                key: "k".to_string(),
                subjects: vec![],
            })
        }));
    catalog.reads = vec![Read::of("github.pull", echo), Read::of("ages.age", age)];
    catalog
}

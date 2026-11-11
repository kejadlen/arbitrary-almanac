//! A catalog for tests: what the binary ships, an event `:e` that hands a script its payload, a
//! `github.pull` that answers with what it was asked, an `ages.age` that counts its calls on the
//! run's [`Ages`] provider, an `ages.cooled` cooldown dated from that count, and two prompts.

use crate::Catalog;
use crate::catalog::{Prompt, Read, Reading};
use almanac::cooled::Published;
use almanac::cx::{Cx, StepError};
use almanac::taint::{Slug, Untrusted};
use futures::future::BoxFuture;
use serde_json::Value as Data;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

/// A fixer's prompt, filled with the repo it repairs.
#[almanac::prelude::prompt(text = "Fix the build of {{ repo }}.")]
#[derive(serde::Deserialize)]
pub struct Fix {
    pub repo: Slug,
}

/// A reader's prompt, with nothing to fill.
#[almanac::prelude::prompt(text = "Read this.")]
#[derive(serde::Deserialize)]
pub struct Look {}

/// A publish time read from the run's [`Ages`]: its calls so far, as days ago, or none for no
/// calls. A release from more than three days back has cooled.
fn published(cx: &Cx, _: Vec<Data>) -> BoxFuture<'_, Result<Published, StepError>> {
    Box::pin(async move {
        let ages = cx.get::<Ages>()?;
        let days = ages.0.load(Ordering::SeqCst);
        if days == 0 {
            return Ok(Published::Unknown);
        }
        let hours = i64::try_from(days * 24).unwrap_or_default();
        let at = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(hours);
        // Almanac's publish time reads RFC 3339, which is how a jiff timestamp prints.
        let at = serde_json::json!({ "At": at.to_string() });
        serde_json::from_value(at).map_err(StepError::from)
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
    catalog.reads = vec![
        Read::of("github.pull", echo),
        Read::of("ages.age", age),
        Read::cooldown("ages.cooled", Duration::from_secs(3 * 86400), published),
    ];
    catalog.prompts = vec![
        Prompt::fixer::<Fix>("fix", |_| "said".to_string()),
        Prompt::quarantined::<Look>("look"),
    ];
    catalog
}

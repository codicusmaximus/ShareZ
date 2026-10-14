//! Small helpers for untyped D-Bus calls with a timeout, so one hung peer can't stall a probe.

use std::time::Duration;

use anyhow::Context;
use serde::{Serialize, de::DeserializeOwned};
use zbus::{
    Connection,
    zvariant::{DynamicType, OwnedValue, Type},
};

pub const TIMEOUT: Duration = Duration::from_millis(1500);

pub async fn call<B, R>(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
    body: &B,
) -> anyhow::Result<R>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type,
{
    let msg = tokio::time::timeout(
        TIMEOUT,
        conn.call_method(Some(dest), path, Some(iface), method, body),
    )
    .await
    .with_context(|| format!("{dest} {iface}.{method} timed out"))?
    .with_context(|| format!("{dest} {iface}.{method}"))?;
    Ok(msg.body().deserialize::<R>()?)
}

pub async fn get_property(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    prop: &str,
) -> anyhow::Result<OwnedValue> {
    call(
        conn,
        dest,
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(iface, prop),
    )
    .await
}

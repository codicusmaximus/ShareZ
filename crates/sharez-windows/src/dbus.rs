use std::time::Duration;

use futures_lite::FutureExt;
use serde::{Serialize, de::DeserializeOwned};
use zbus::{Connection, zvariant::DynamicType};

use crate::{Error, Result};

/// Per-call timeout. Accessibility peers in particular can be unresponsive apps.
const TIMEOUT: Duration = Duration::from_millis(1500);

pub async fn call<B, R>(
    conn: &Connection,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
    body: &B,
) -> Result<R>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + zbus::zvariant::Type,
{
    let call = async {
        let reply = conn
            .call_method(Some(dest), path, Some(iface), method, body)
            .await?;
        Ok(reply.body().deserialize::<R>()?)
    };
    let timeout = async {
        async_io::Timer::after(TIMEOUT).await;
        Err(Error::Timeout("a D-Bus peer"))
    };
    call.or(timeout).await
}

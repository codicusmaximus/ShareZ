//! Shared AT-SPI plumbing: the accessibility bus and the few calls ShareZ needs on it.

use sharez_core::Rect;
use zbus::{Connection, zvariant::OwnedObjectPath};

use crate::{Error, Result, dbus::call};

pub const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
pub const COMPONENT: &str = "org.a11y.atspi.Component";
pub const COORDS_SCREEN: u32 = 0;
pub const COORDS_WINDOW: u32 = 1;

/// AT-SPI `StateType` indices.
const STATE_SHOWING: u32 = 25;

/// An accessible object: its owner's bus name and its object path.
pub type Node = (String, OwnedObjectPath);

pub fn registry_root() -> Node {
    (
        "org.a11y.atspi.Registry".into(),
        OwnedObjectPath::try_from("/org/a11y/atspi/accessible/root").expect("valid path"),
    )
}

pub fn is_null(node: &Node) -> bool {
    node.1.as_str() == "/org/a11y/atspi/null"
}

/// Turns accessibility on for the session if it is off. Apps only publish their trees while it
/// is on; ones started earlier may need a restart to appear.
pub async fn ensure_enabled() -> Result<()> {
    let session = Connection::session().await?;
    let proxy =
        zbus::Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status").await?;
    if !proxy.get_property::<bool>("IsEnabled").await? {
        proxy
            .set_property("IsEnabled", true)
            .await
            .map_err(zbus::Error::from)?;
    }
    Ok(())
}

pub async fn bus() -> Result<Connection> {
    let session = Connection::session().await?;
    let addr: String = call(
        &session,
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
        &(),
    )
    .await?;
    Ok(zbus::connection::Builder::address(addr.as_str())?
        .build()
        .await?)
}

pub async fn children(conn: &Connection, node: &Node) -> Result<Vec<Node>> {
    call(
        conn,
        &node.0,
        node.1.as_str(),
        ACCESSIBLE,
        "GetChildren",
        &(),
    )
    .await
}

pub async fn role(conn: &Connection, node: &Node) -> String {
    call(
        conn,
        &node.0,
        node.1.as_str(),
        ACCESSIBLE,
        "GetRoleName",
        &(),
    )
    .await
    .unwrap_or_default()
}

pub async fn name(conn: &Connection, node: &Node) -> String {
    let v: Result<zbus::zvariant::OwnedValue> = call(
        conn,
        &node.0,
        node.1.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &(ACCESSIBLE, "Name"),
    )
    .await;
    v.ok()
        .and_then(|v| String::try_from(v).ok())
        .unwrap_or_default()
}

pub async fn showing(conn: &Connection, node: &Node) -> bool {
    let states: Result<Vec<u32>> =
        call(conn, &node.0, node.1.as_str(), ACCESSIBLE, "GetState", &()).await;
    states.is_ok_and(|s| {
        s.get((STATE_SHOWING / 32) as usize)
            .is_some_and(|word| word & (1 << (STATE_SHOWING % 32)) != 0)
    })
}

pub async fn extents(conn: &Connection, node: &Node, coords: u32) -> Result<Rect> {
    let (x, y, w, h): (i32, i32, i32, i32) = call(
        conn,
        &node.0,
        node.1.as_str(),
        COMPONENT,
        "GetExtents",
        &(coords,),
    )
    .await?;
    if x == i32::MIN || w <= 0 || h <= 0 {
        return Err(Error::BadData(
            "AT-SPI",
            "element has no on-screen extents".into(),
        ));
    }
    Ok(Rect::new(x.into(), y.into(), w.into(), h.into()))
}

pub async fn accessible_at(conn: &Connection, node: &Node, x: i32, y: i32) -> Result<Node> {
    call(
        conn,
        &node.0,
        node.1.as_str(),
        COMPONENT,
        "GetAccessibleAtPoint",
        &(x, y, COORDS_WINDOW),
    )
    .await
}

pub async fn parent(conn: &Connection, node: &Node) -> Result<Node> {
    let v: zbus::zvariant::OwnedValue = call(
        conn,
        &node.0,
        node.1.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &(ACCESSIBLE, "Parent"),
    )
    .await?;
    let (bus, path): (String, OwnedObjectPath) = v
        .try_into()
        .map_err(|e: zbus::zvariant::Error| Error::BadData("AT-SPI", e.to_string()))?;
    Ok((bus, path))
}

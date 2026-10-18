//! Wires `imc-support`'s `EndpointBridge` into the `imc-trace`
//! [`RouteBridge`] trait — the Rust equivalent of Python's
//! `ASTTracer.__init__`, which always builds
//! `EndpointBridge(Path(project.path), symbol_defs)` for an indexed project.
//!
//! Without this adapter `imc trace` never sees HTTP routes: `GET /api/x`
//! targets, frontend fetch/axios call sites and RTK Query hooks all resolve to
//! nothing, which is exactly what the bridge-less tracer reports today.
//!
//! Interior mutability is required because [`RouteBridge`] takes `&self` while
//! `EndpointBridge` scans lazily behind `&mut self`. Nothing here re-enters the
//! bridge, so a `RefCell` borrow is always short-lived.

use std::cell::RefCell;
use std::path::Path;

use imc_core::store::Store;
use imc_support::endpoints::{ClientApiCall, EndpointBridge, RouteDefinition};
use imc_trace::tree::{ClientCall, HttpRoute, RouteBridge};

/// `RouteBridge` over a real [`EndpointBridge`] rooted at the project.
pub struct EndpointRouteBridge<'a> {
    inner: RefCell<EndpointBridge<'a>>,
}

impl<'a> EndpointRouteBridge<'a> {
    /// `store` supplies handler node ids so a route trace can jump into the
    /// backend handler; `None` still reports routes, but with unresolved ids.
    pub fn new(root_dir: &Path, store: Option<&'a Store>) -> Self {
        Self {
            inner: RefCell::new(EndpointBridge::new(root_dir, store)),
        }
    }
}

/// Python treats an empty `caller_file` as "no file filter"
/// (`if caller_file:`); passing `Some("")` here would filter every call site
/// out instead.
fn file_scope(file: &str) -> Option<&str> {
    (!file.is_empty()).then_some(file)
}

fn http_route(r: RouteDefinition) -> HttpRoute {
    HttpRoute {
        method: r.method,
        path: r.path,
        normalized_path: r.normalized_path,
        handler_node_id: r.handler_node_id,
        handler_name: r.handler_name,
        file: r.file,
        line: Some(r.line),
    }
}

fn client_call(c: ClientApiCall) -> ClientCall {
    ClientCall {
        caller_name: c.caller_name,
        caller_node_id: c.caller_node_id,
        file: c.file,
        line: Some(c.line),
        raw_url: c.raw_url,
        method: c.method,
        normalized_path: c.normalized_path,
    }
}

impl RouteBridge for EndpointRouteBridge<'_> {
    fn routes_for_target(&self, method: &str, path: &str) -> Vec<HttpRoute> {
        self.inner
            .borrow_mut()
            .get_routes_for_target(method, path)
            .into_iter()
            .map(http_route)
            .collect()
    }

    fn routes_for_caller(
        &self,
        caller_name: &str,
        caller_file: &str,
    ) -> Vec<(ClientCall, Vec<HttpRoute>)> {
        self.inner
            .borrow_mut()
            .get_routes_for_caller(caller_name, file_scope(caller_file))
            .into_iter()
            .map(|(call, routes)| {
                (
                    client_call(call),
                    routes.into_iter().map(http_route).collect(),
                )
            })
            .collect()
    }

    fn client_callers_for_handler(
        &self,
        handler_name: &str,
        handler_file: &str,
    ) -> Vec<(HttpRoute, Vec<ClientCall>)> {
        self.inner
            .borrow_mut()
            .get_client_callers_for_handler(handler_name, file_scope(handler_file))
            .into_iter()
            .map(|(route, calls)| {
                (
                    http_route(route),
                    calls.into_iter().map(client_call).collect(),
                )
            })
            .collect()
    }

    fn client_calls_by_caller(&self, caller_name: &str) -> Vec<ClientCall> {
        self.inner
            .borrow_mut()
            .client_calls(caller_name)
            .into_iter()
            .map(client_call)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "imc-routebridge-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn route_target_lookup_uses_module_prefix() {
        let root = tmpdir("target");
        write(
            &root,
            "api/git.py",
            "from fastapi import APIRouter\n\n\
             router = APIRouter(prefix=\"/api/git\")\n\n\
             @router.get(\"/status\")\n\
             async def status():\n\
             \x20   return {}\n",
        );
        let bridge = EndpointRouteBridge::new(&root, None);
        let routes = bridge.routes_for_target("GET", "/api/git/status");
        assert_eq!(routes.len(), 1, "{routes:?}");
        assert_eq!(routes[0].path, "/api/git/status");
        assert_eq!(routes[0].handler_name, "status");
        assert_eq!(routes[0].file, "api/git.py");
        // An unrelated path must miss. (`/status` itself would match the
        // prefixed route by `match_routes`' deliberate suffix rule, exactly as
        // in Python — that is how a bare frontend path links to a prefixed
        // backend route, so it is not an assertion to make here.)
        assert!(bridge.routes_for_target("GET", "/totally/missing").is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn client_calls_resolve_frontend_fetch_sites() {
        let root = tmpdir("calls");
        write(
            &root,
            "src/App.jsx",
            "export function App() {\n  return fetch('/api/items');\n}\n",
        );
        let bridge = EndpointRouteBridge::new(&root, None);
        let calls = bridge.client_calls_by_caller("App");
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].normalized_path, "/api/items");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_file_scope_means_unfiltered() {
        let root = tmpdir("scope");
        write(
            &root,
            "src/App.jsx",
            "export function App() {\n  return fetch('/api/items');\n}\n",
        );
        let bridge = EndpointRouteBridge::new(&root, None);
        // An empty file must not filter everything out (Python: `if caller_file:`).
        let pairs = bridge.routes_for_caller("App", "");
        assert_eq!(pairs.len(), 1, "{pairs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! Canonical public-route definitions for the AMATL server.
//!
//! Each entry below generates both an Axum registration and the inventory used
//! by the OpenAPI coverage guard. Adding a route therefore requires declaring
//! its path and methods in one place.

use super::*;
#[cfg(test)]
use http::Method;

/// A public route definition consisting of a path and supported HTTP methods.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg(test)]
pub struct Route {
    pub path: &'static str,
    pub methods: &'static [Method],
}

macro_rules! define_public_routes {
    ($( $path:literal => [$($method:ident($handler:ident)),+ $(,)?] ),+ $(,)?) => {
        pub(super) fn register_public_routes(router: Router<AppState>) -> Router<AppState> {
            router$(.route(
                $path,
                axum::routing::MethodRouter::new()$(.$method($handler))+
            ))+
        }

        /// Returns the route inventory generated from the router declarations.
        #[cfg(test)]
        pub fn all_routes() -> Vec<Route> {
            vec![$(Route {
                path: $path,
                methods: &[$(define_public_routes!(@method $method)),+],
            }),+]
        }
    };
    (@method get) => { Method::GET };
    (@method post) => { Method::POST };
    (@method patch) => { Method::PATCH };
    (@method delete) => { Method::DELETE };
}

define_public_routes! {
    "/search" => [get(search), post(search_post)],
    "/deep" => [get(deep), post(deep_post)],
    "/answer" => [post(answer_post), patch(update_answer_fields)],
    "/providers" => [get(providers)],
    "/status" => [get(status)],
    "/history" => [get(history), delete(purge_history)],
    "/history/{id}" => [delete(delete_history_entry)],
    "/saved" => [get(saved_documents), post(save_document)],
    "/saved/{id}" => [delete(delete_saved_document)],
    "/reload" => [post(reload)],
    "/answer/enabled" => [post(answer_toggle)],
    "/providers/{name}/enabled" => [post(provider_toggle)],
    "/providers/{name}" => [get(provider_record), patch(update_provider_record)],
    "/inference" => [patch(update_inference)],
    "/server/clients" => [get(list_server_clients), post(create_server_client)],
    "/server/clients/{id}" => [patch(update_server_client), delete(delete_server_client)],
    "/server/clients/{id}/rotate" => [post(rotate_server_client_token)],
    "/server/pending-config" => [get(server_pending_config), patch(update_server_pending_config)],
    "/data-policy" => [post(data_policy_update)],
    "/policies" => [get(policies)],
    "/policies/{name}" => [patch(update_policy)],
    "/persistence" => [get(persistence_config), patch(update_persistence)],
    "/persistence/backups" => [get(list_backups)],
    "/persistence/backup" => [post(create_backup)],
    "/circuits" => [get(circuits)],
    "/circuits/reset" => [post(reset_circuits)],
    "/telemetry" => [get(telemetry_config), patch(update_telemetry)],
    // `/deep` is the domain deep-fetch surface; configuration is below it.
    "/deep/limits" => [get(deep_config), patch(update_deep)],
    "/deep/extractor" => [patch(update_deep_extractor)],
    "/deep/renderer" => [patch(update_deep_renderer)],
    "/security-events" => [get(security_events)],
    "/health" => [get(health)],
    "/ready" => [get(ready)],
    "/metrics" => [get(metrics)],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_routes_has_unique_paths_and_methods() {
        let routes = all_routes();
        let mut seen = std::collections::HashSet::new();

        for route in routes {
            assert!(
                !route.methods.is_empty(),
                "Route {} has no methods",
                route.path
            );
            for method in route.methods {
                assert!(
                    seen.insert((route.path, method.clone())),
                    "Duplicate operation {:?} {}",
                    method,
                    route.path
                );
            }
        }
    }
}

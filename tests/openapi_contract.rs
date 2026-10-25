use serde_json::Value;

fn spec() -> Value {
    serde_json::from_str(include_str!("../docs/openapi-v1.json")).expect("valid OpenAPI JSON")
}

#[test]
fn required_v1_endpoints_are_documented_with_the_expected_methods() {
    let document = spec();
    let paths = document["paths"].as_object().expect("paths object");
    let required = [
        ("/api/v1/auth/login", "post"),
        ("/api/v1/auth/me", "get"),
        ("/api/v1/auth/logout", "post"),
        ("/api/v1/customers", "get"),
        ("/api/v1/customers/{id}", "get"),
        ("/api/v1/analytics/summary", "get"),
        ("/api/v1/model/settings", "get"),
        ("/api/v1/model/settings", "patch"),
        ("/api/v1/source/snapshot", "put"),
        ("/metrics", "get"),
        ("/api/v1/docs", "get"),
    ];
    for (path, method) in required {
        assert!(
            paths
                .get(path)
                .is_some_and(|operations| operations.get(method).is_some()),
            "missing {method} {path}"
        );
    }
}

#[test]
fn customer_sorting_pagination_and_source_statuses_are_in_the_contract() {
    let document = spec();
    let parameters = document["paths"]["/api/v1/customers"]["get"]["parameters"]
        .as_array()
        .expect("customer query parameters");
    let sort = parameters
        .iter()
        .find(|parameter| parameter["name"] == "sort")
        .unwrap();
    let fields: Vec<_> = sort["schema"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for required in ["id", "email", "balance", "totalReplenished", "score"] {
        assert!(
            fields.contains(&required),
            "customer sort field {required} is undocumented"
        );
    }
    assert!(
        parameters
            .iter()
            .any(|parameter| parameter["name"] == "page")
    );
    assert!(
        parameters
            .iter()
            .any(|parameter| parameter["name"] == "limit")
    );

    let service_statuses: Vec<_> =
        document["components"]["schemas"]["Service"]["properties"]["status"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
    for required in ["active", "suspended", "prolong_wait", "deleted", "blocked"] {
        assert!(
            service_statuses.contains(&required),
            "service status {required} is undocumented"
        );
    }
    let transaction = &document["components"]["schemas"]["Transaction"]["properties"];
    let statuses: Vec<_> = transaction["status"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let types: Vec<_> = transaction["type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(statuses, vec!["performed", "not_performed"]);
    assert_eq!(types, vec!["prolong", "order"]);
}

#[test]
fn protected_customer_and_settings_operations_document_session_authentication() {
    let document = spec();
    for (path, method) in [
        ("/api/v1/auth/me", "get"),
        ("/api/v1/customers", "get"),
        ("/api/v1/customers/{id}", "get"),
        ("/api/v1/analytics/summary", "get"),
        ("/api/v1/model/settings", "get"),
        ("/api/v1/model/settings", "patch"),
        ("/api/v1/source/snapshot", "put"),
    ] {
        assert!(
            document["paths"][path][method]["security"]
                .as_array()
                .is_some_and(|security| !security.is_empty()),
            "{method} {path} should require a session"
        );
    }
}

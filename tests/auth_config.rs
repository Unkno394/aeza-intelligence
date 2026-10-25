use serde_json::Value;

fn check_realm_config(path: &str) {
    let config: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(config["realm"], "aeza");
    assert_eq!(config["enabled"], true);
    assert_eq!(config["registrationAllowed"], false);
    assert_eq!(config["bruteForceProtected"], true);

    let client = config["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|client| client["clientId"] == "aeza-intelligence")
        .expect("realm must import the application OIDC client");
    assert_eq!(client["enabled"], true);
    assert_eq!(client["protocol"], "openid-connect");
    assert_eq!(client["standardFlowEnabled"], true);
    assert_ne!(client["directAccessGrantsEnabled"], true);
    assert_ne!(client["implicitFlowEnabled"], true);
    assert_eq!(client["attributes"]["pkce.code.challenge.method"], "S256");
    assert!(
        client["redirectUris"]
            .as_array()
            .unwrap()
            .iter()
            .any(|uri| uri
                .as_str()
                .is_some_and(|uri| uri.ends_with("/api/auth/callback")))
    );
}

#[test]
fn development_and_deployment_realms_share_oidc_and_brute_force_requirements() {
    check_realm_config("dev/keycloak/aeza-realm.json");
    check_realm_config("keycloak/aeza-realm.json");
}

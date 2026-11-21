use osdl_core::OsdlConfig;

#[test]
fn qx96m_is_an_engine_recipe_instrument() {
    let config: OsdlConfig = serde_yaml::from_str(
        "
qx96m_instruments:
  - id: jlm-qpcr
    endpoint: 192.168.42.10:6666
    key_file: private/qx96m.key
",
    )
    .unwrap();
    config.qx96m_instruments[0]
        .validate()
        .expect("private endpoint and key file");
    let round_trip = serde_yaml::to_value(config).unwrap();
    assert_eq!(
        round_trip["qx96m_instruments"][0]["id"].as_str(),
        Some("jlm-qpcr")
    );
    assert_eq!(
        round_trip["qx96m_instruments"][0]["key_file"].as_str(),
        Some("private/qx96m.key")
    );
}

#[test]
fn qx96m_configuration_requires_a_private_endpoint_and_secret_reference() {
    use osdl_core::config::Qx96mConfig;

    let public = serde_yaml::from_str::<Qx96mConfig>(
        "id: station\nendpoint: 203.0.113.10:6666\nkey_file: private/qx96m.key\n",
    )
    .unwrap();
    assert!(public.validate().is_err());
    let inline = serde_yaml::from_str::<Qx96mConfig>(
        "id: station\nendpoint: 192.168.42.10:6666\nkey_file: private/qx96m.key\nprotocol_key: fixture!\n",
    );
    assert!(inline.is_err());
}

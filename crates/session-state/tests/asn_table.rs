use session_state::{NetKind, asn_info};

#[test]
fn asn_table_maps_kinds() {
    assert_eq!(asn_info(15169).net, NetKind::Datacenter);
    assert_eq!(asn_info(7922).net, NetKind::Residential);
    assert_eq!(asn_info(7018).net, NetKind::Mobile);
    assert_eq!(asn_info(999999).net, NetKind::Residential);
    assert_eq!(asn_info(3209).locale, "de-DE");
}

use super::*;

fn sdp() -> String {
    "v=0\r\na=fingerprint:sha-256 00:11\r\n".into()
}

#[test]
fn signaling_is_closed_bounded_and_requires_dtls_binding() {
    assert!(Signal::Offer { sdp: sdp() }.valid());
    assert!(Signal::Answer { sdp: sdp() }.valid());
    for sdp in [
        "v=0\r\n".into(),
        "v=0\0\r\na=fingerprint:x".into(),
        "x".repeat(65 * 1024),
    ] {
        assert!(!Signal::Offer { sdp }.valid());
    }
    let candidate = |value: String| Signal::Candidate {
        candidate: value,
        sdp_mid: Some("data".into()),
        sdp_m_line_index: Some(0),
        username_fragment: None,
    };
    assert!(candidate("candidate:1 1 UDP 1 192.0.2.1 9999 typ host".into()).valid());
    for value in [
        "".into(),
        "candidate:bad\r\n".into(),
        format!("candidate:{}", "x".repeat(2048)),
    ] {
        assert!(!candidate(value).valid());
    }
    let base = json!({"type":"rtc.signal","negotiationId":Uuid::new_v4(),"sessionId":Uuid::new_v4(),"hostDeviceId":Uuid::new_v4(),"sequence":0,"signal":{"type":"offer","sdp":sdp()},"signature":"a".repeat(86)});
    assert!(serde_json::from_value::<SignalFrame>(base.clone()).is_ok());
    let mut spoof = base.clone();
    spoof["fromDeviceId"] = json!(Uuid::new_v4());
    assert!(serde_json::from_value::<SignalFrame>(spoof).is_err());
    let mut command = base;
    command["signal"] = json!({"type":"command","operation":"points.adjust"});
    assert!(serde_json::from_value::<SignalFrame>(command).is_err());
}

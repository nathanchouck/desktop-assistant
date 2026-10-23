use super::*;

#[test]
fn pkce_matches_the_rfc_example() {
    // RFC 7636 appendix B.
    let bytes: [u8; 32] = [
        116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212, 37,
        77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
    ];
    let pkce = Pkce::from_bytes(&bytes);
    assert_eq!(pkce.verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(
        pkce.challenge,
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    assert_eq!(format!("{pkce:?}"), "Pkce(***)");
    let fresh = Pkce::generate().unwrap();
    assert_eq!(fresh.verifier.len(), 43);
    assert_ne!(fresh.verifier, Pkce::generate().unwrap().verifier);
    assert_eq!(random_state().unwrap().len(), 22);
}

#[test]
fn the_consent_page_address() {
    let url = authorization_url(
        "123-abc.apps.googleusercontent.com",
        "http://127.0.0.1:51234",
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        "st4te",
    );
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?client_id=123-abc.apps.googleusercontent.com&"));
    for part in [
        "redirect_uri=http%3A%2F%2F127.0.0.1%3A51234",
        "response_type=code",
        "scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.events.readonly%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.metadata%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.metadata.readonly",
        "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        "code_challenge_method=S256",
        "state=st4te",
        "access_type=offline",
        "prompt=consent",
    ] {
        assert!(url.contains(part), "{part} missing from {url}");
    }
}

#[test]
fn the_redirect_must_carry_our_state() {
    assert_eq!(
        parse_callback("state=abc&code=4%2F0AQ-x_y&scope=email", "abc"),
        Ok(Callback::Code("4/0AQ-x_y".into()))
    );
    assert_eq!(
        parse_callback("?error=access_denied&state=abc", "abc"),
        Ok(Callback::Refused("access_denied".into()))
    );
    assert_eq!(
        parse_callback("state=abd&code=4%2F0", "abc"),
        Err(CallbackError::WrongState)
    );
    assert_eq!(
        parse_callback("code=4%2F0", "abc"),
        Err(CallbackError::WrongState)
    );
    assert_eq!(
        parse_callback("state=ab&code=x", "abc"),
        Err(CallbackError::WrongState)
    );
    assert_eq!(parse_callback("", "abc"), Err(CallbackError::NotACallback));
    assert_eq!(
        parse_callback("state=abc&code=", "abc"),
        Err(CallbackError::NotACallback)
    );
}

#[test]
fn decodes_query_values() {
    assert_eq!(decode("a%20b+c%2F"), "a b c/");
    assert_eq!(decode("100%"), "100%");
    assert_eq!(decode("%zz"), "%zz");
    assert_eq!(decode("%C3%A9"), "é");
    assert_eq!(decode("%é%4"), "%é%4");
}

#[test]
fn token_forms() {
    let form = code_exchange_form(
        "4/0AQ",
        "id.apps",
        &Secret::new("GOCSPX-s"),
        "http://127.0.0.1:5",
        "verif",
    );
    assert_eq!(
        form,
        "code=4%2F0AQ&client_id=id.apps&client_secret=GOCSPX-s&redirect_uri=http%3A%2F%2F127.0.0.1%3A5&grant_type=authorization_code&code_verifier=verif"
    );
    assert_eq!(
        refresh_form(&Secret::new("1//r"), "id", &Secret::new("s")),
        "refresh_token=1%2F%2Fr&client_id=id&client_secret=s&grant_type=refresh_token"
    );
}

/// An ID token with `{"email":"me@example.com"}` as its claims.
fn id_token() -> String {
    let claims = URL_SAFE_NO_PAD.encode(
        br#"{"iss":"https://accounts.google.com","email":"me@example.com","email_verified":true}"#,
    );
    format!("eyJhbGciOiJSUzI1NiJ9.{claims}.c2ln")
}

#[test]
fn reads_a_first_exchange() {
    let body = format!(
        r#"{{"access_token":"ya29.a0","expires_in":3599,"refresh_token":"1//0g","scope":"openid {CALENDAR_SCOPE} email","token_type":"Bearer","id_token":"{}"}}"#,
        id_token()
    );
    let tokens = parse_tokens(200, body.as_bytes(), 1_000).unwrap();
    assert_eq!(tokens.access.expose(), "ya29.a0");
    assert_eq!(tokens.expires_at, 4_599);
    assert_eq!(tokens.refresh.as_ref().map(Secret::expose), Some("1//0g"));
    assert_eq!(tokens.email.as_deref(), Some("me@example.com"));
    let granted = Granted::from_scope(tokens.scope.as_deref());
    assert_eq!(
        granted,
        Granted {
            calendar: true,
            mail: false,
            drive: false
        }
    );
    // Tokens never show up in logs.
    assert!(!format!("{tokens:?}").contains("ya29"));
}

#[test]
fn reads_a_refresh_and_errors() {
    let tokens = parse_tokens(200, br#"{"access_token":"ya29.b","expires_in":3599}"#, 0).unwrap();
    assert!(tokens.refresh.is_none() && tokens.email.is_none());
    assert_eq!(
        Granted::from_scope(None),
        Granted {
            calendar: true,
            mail: true,
            drive: true
        }
    );

    let dead = parse_tokens(
        400,
        br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        0,
    )
    .unwrap_err();
    assert!(dead.needs_reconnect());
    assert!(dead.to_string().contains("Connect again"), "{dead}");
    let other = parse_tokens(
        400,
        br#"{"error":"invalid_request","error_description":"Missing code verifier."}"#,
        0,
    )
    .unwrap_err();
    assert!(!other.needs_reconnect());
    assert_eq!(
        other.to_string(),
        "Google refused: Missing code verifier. (invalid_request)"
    );
    assert!(matches!(
        parse_tokens(502, b"<html>", 0),
        Err(TokenError::Unreadable(_))
    ));
    assert!(matches!(
        parse_tokens(200, b"{}", 0),
        Err(TokenError::Unreadable(_))
    ));
    assert!(parse_tokens(401, br#"{"error":"invalid_client"}"#, 0)
        .unwrap_err()
        .needs_reconnect());
}

#[test]
fn refreshes_a_minute_early() {
    assert!(!needs_refresh(1_000, 900));
    assert!(needs_refresh(1_000, 940));
    assert!(needs_refresh(1_000, 2_000));
}

#[test]
fn email_from_odd_tokens() {
    assert_eq!(
        email_from_id_token(&id_token()).as_deref(),
        Some("me@example.com")
    );
    assert_eq!(email_from_id_token("not-a-jwt"), None);
    assert_eq!(email_from_id_token("a.!!!.c"), None);
    let no_email = URL_SAFE_NO_PAD.encode(br#"{"sub":"1"}"#);
    assert_eq!(email_from_id_token(&format!("a.{no_email}.c")), None);
}

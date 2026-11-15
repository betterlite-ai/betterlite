use super::*;

const RSA_PKCS1_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----
MIIEpQIBAAKCAQEAzU916d6LKEjL1X+jrpZ2EiMw3q0I3CtqXI/Rwb2T3Qrcx80M
GGllokCBukVzp44OU7B3MAjtK7Djb4B6fLi8t0+EgbsbcO72YenYwuLgA9p3TS/0
o+9fFRh4NK4CsEbi6i2uh6iZmsJUnx4GVdfi/Ik5Kto4doNJAJCB0Cj5yTy0VJxh
qpGmLD03vwRErL+1TMchUH3aXJ0ThU96SwjoyNZmmsUuLztaS5454zWQFHnx4DE5
Hls7eqKSuvP7ZheNTavxZ6m3ypkJ3ru4QilwcAATjIv95XSpGYoRkg2Dd6MlWjmo
UR+rLEXy/TyTW1H0brksrU7AqJNfPqcGSBf4OwIDAQABAoIBAFl1Yh+BRIlNDuP8
NcoA7XmBs+2S3+VvVWz3jaIphKAv93tnsPmCEzLbgKs1T+uWTk/Q+F4Jxa3f6XnQ
NY7av//vugD9EjvnI5wOVNC2In4LVyhe2O4FgxyiCN6LY8iDWqRHO2EI6cUxH8Yl
QwVsiA8Zi7tMUoXxNoH0dVvKPuybqoCz0DiIp39H2ZqFgNxHEcE6A+bxnWmRWhjF
V5JvpBQNa9qPrjKuWJB5rU+5GJCbNltA1kGEv9MOGmHdvCCtvbqCNKsxMHftNpLM
mf93sFM/ppA1GNR9wST9g7BxnCc1SSS75zQyXosuZUTBmg3eCy3Ddcn3jgN0qbzt
xukh4WECgYEA9dfR6K/D3ODpDRGUiOdVi4B61Bl0+C6nhUNff8yasH54mVYw9Rzd
UoCz5NV/UZZaiWA/XWwubs8oMXsq8kIngi4iJYaZetTgzVnBRh9VbIvF4H3SrRoq
uGqwg2GsDDloW8KxtNSPSCihAVqOKREE8INz0zVSxRHGN1n3mpG1G+cCgYEA1crx
jQ7ON2IRyHkpWNrlJIYowT9N+zd5F0HVufLx1kojR7pPOQPEhfTZyPluBVFM7Ev0
tqHVAPRc4/4gMSNGg8B5xk70qlImaOnJT7XfUBD65DQ8X02K8EOSFOUi2ZkVmSC4
N+fE98fk6bfZNMHIipvIa2K1KSOrOOBZwRFIVo0CgYEA3z/f1QhYOyrbOAzw7+nz
2z0PTbDYcdAcPH5UKQAqsYAIEGcfClft/E/4DSR7/CEnkn0ezb3B5MqmW1jYRSsi
KauvQpiao9rRUmnhg8ZMksfTJ+va92s9xD7OkGCQFoHDbSC+s9IbU5HhtBMB6fUU
oT0eX3ya728Wq4k9utX6u50CgYEAj4+/tGEEyNvp5Ry3TxtuU6BjLaE81Dzj62Ii
CsPA8YzHNBs/p6GIDgvlUrdr0dZOOT6oDRuO1R1CYzb6lwSBDWfqXegXE2gmsryK
miQe2L0OPL926ug/LCezqKRVSpkpj7IRAjl4YoqwhOEX0f1ulOUXKEUYMY/qIj9F
Bgh88uECgYEA1/lTpKtG6pkFILG2UtexIcAmfn8l5F+mqGKRHOvGI8QMvsVxJu6o
qC/lXpzk6xWsaVt8VchMgUafB5pWUKc2dOEg+idaxTUYsTKVko5tP7G9DOsqB9Jb
awsw2jwjSBwoFyOFAQ9uGh6a9sPAns2suhzFWCy0LU+KsHF0b9WY6f0=
-----END RSA PRIVATE KEY-----
";

const EC_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgjWnEOhxiUcahHIlQ
yseMVYYpoKrzUlA6Q+i/M7F8bfehRANCAATfEGU2LBPLuyGI25RO/AlHvoq447WR
V90yMvst0vNODYv/OYCpV+lYgX5HD/PeEBOAipgimyxVtkamIv1X1QVL
-----END PRIVATE KEY-----
";

const ED_KEY: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIKs0MB1rmJeaWaRqB9QDIcokFBas4VPS1CU5g6x55K6V
-----END PRIVATE KEY-----
";

const SEC1_EC_KEY: &str = "-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIHUBLcJcUQ76z8Mxd9QdRm3plOGAsFCOVF2FnDzSMSOaoAoGCCqGSM49
AwEHoUQDQgAESufTWoe21/tLLkwuAHaXYj+A/6dkJ1SIXqthbZXZkjeFeWcj6k+F
zXpk6sw1TlgYMHVUZNzf2sZGo6Ewde0CeQ==
-----END EC PRIVATE KEY-----
";

#[tokio::test]
async fn signs_with_a_pkcs1_rsa_key() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("RS256", RSA_PKCS1_KEY, None),
        )
        .await
        .unwrap();
    let (header, claims) = read_assertion(post.form().get("client_assertion").unwrap());
    assert_eq!(header.alg, "RS256");
    assert_eq!(claims.aud, ENDPOINT);
}

#[tokio::test]
async fn signs_with_an_ec_key() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("ES256", EC_KEY, Some("kid-ec")),
        )
        .await
        .unwrap();
    let (header, claims) = read_assertion(post.form().get("client_assertion").unwrap());
    assert_eq!(header.alg, "ES256");
    assert_eq!(header.kid.as_deref(), Some("kid-ec"));
    assert_eq!(claims.iss, CLIENT_ID);
}

#[tokio::test]
async fn signs_with_an_ed_key() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("EdDSA", ED_KEY, None),
        )
        .await
        .unwrap();
    let (header, claims) = read_assertion(post.form().get("client_assertion").unwrap());
    assert_eq!(header.alg, "EdDSA");
    assert_eq!(claims.exp - claims.iat, 60);
}

#[tokio::test]
async fn rejects_an_sec1_ec_key() {
    let (client, post) = client(vec![answered(200, r#"{"access_token":"minted"}"#)]);
    let error = client
        .fetch(
            ENDPOINT,
            CLIENT_ID,
            &grant(),
            &jwt_auth("ES256", SEC1_EC_KEY, None),
        )
        .await
        .unwrap_err();
    assert_eq!(error, CredError::Misconfigured(SIGNING_FAILED.to_owned()));
    assert!(post.forms.lock().unwrap().is_empty());
}

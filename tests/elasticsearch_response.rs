use lazydb::{
    agent::policy::authorize_query, db::elasticsearch::ElasticsearchRequest,
    profile::import_connection_url,
};

#[test]
fn read_policy_rejects_unknown_stateful_and_ambiguous_paths() {
    for url in ["elasticsearch://localhost", "opensearch://localhost"] {
        let profile = import_connection_url(url, None).unwrap().profile;
        for request in [
            "HEAD /books",
            "HEAD /books/_doc/1",
            "HEAD /books/_alias/current",
            "GET /books/_mapping",
            "GET /_cat/indices?format=json",
            "POST /books/_search\n{}",
            "POST /books/_msearch\n{}\n{}",
            "GET /_cluster/health",
            "GET /books/_doc/1",
            "POST /books/_validate/query\n{}",
        ] {
            assert!(
                ElasticsearchRequest::parse(request).unwrap().is_read_only(),
                "{request}"
            );
            assert!(authorize_query(&profile, request).is_ok(), "{request}");
        }
        for request in [
            "HEAD /_unknown",
            "HEAD /books/../_refresh",
            "HEAD /books/%2e%2e/_refresh",
            "HEAD /books\n{}",
            "GET /books/_doc/1?refresh=true",
            "HEAD /books/_doc/1?refresh=true",
            "POST /books/_mget?%72efresh=true\n{}",
            "GET /_search/scroll",
            "POST /_search/scroll\n{}",
            "POST /books/_pit?keep_alive=1m",
            "POST /books/_search?scroll=1m\n{}",
            "GET /books/_search?%73croll=1m",
            "POST /_msearch?scroll=1m\n{}\n{}",
            "POST /books/_refresh",
            "GET /unknown/path/_mapping",
            "GET /books/%5fsearch",
            "GET /books/./_search",
            "POST /books/_doc/_search\n{}",
            "POST /books/_delete_by_query\n{}",
            "POST /books/_bulk\n{}",
            "DELETE /books",
            "GET /_unknown",
        ] {
            assert!(
                !ElasticsearchRequest::parse(request).is_ok_and(|request| request.is_read_only()),
                "{request}"
            );
            assert!(authorize_query(&profile, request).is_err(), "{request}");
        }
    }
}

use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::*;

#[test]
fn test_dead_link() {
    let site = assert_fs::TempDir::new().unwrap();
    site.child("index.html")
        .write_str("<a href=bar.html>")
        .unwrap();
    let mut cmd = Command::cargo_bin("hyperlink").unwrap();
    cmd.current_dir(site.path()).arg(".");

    cmd.assert().failure().code(1).stdout(
        predicate::str::is_match(
            r#"^Reading files
Checking 1 links from 1 files \(1 documents\)
\..index\.html
  error: bad link /bar.html

Found 1 bad links
"#,
        )
        .unwrap(),
    );
    site.close().unwrap();
}

#[test]
fn test_dead_anchor() {
    let site = assert_fs::TempDir::new().unwrap();
    site.child("index.html")
        .write_str("<a href=bar.html#goo>")
        .unwrap();
    site.child("bar.html").touch().unwrap();
    let mut cmd = Command::cargo_bin("hyperlink").unwrap();
    cmd.current_dir(site.path()).arg(".").arg("--check-anchors");

    cmd.assert().failure().code(2).stdout(
        predicate::str::is_match(
            r#"^Reading files
Checking 1 links from 2 files \(2 documents\)
\..index\.html
  error: bad link /bar.html#goo

Found 0 bad links
Found 1 bad anchors
$"#,
        )
        .unwrap(),
    );
    site.close().unwrap();
}

#[test]
fn test_bad_dir() {
    let mut cmd = Command::cargo_bin("hyperlink").unwrap();
    cmd.arg("non_existing_dir");

    cmd.assert()
        .failure()
        .code(1)
        .stdout("Reading files\n")
        .stderr(predicate::str::contains(
            "Error: IO error for operation on non_existing_dir:",
        ));
}

#[test]
fn test_redirects() {
    let site = assert_fs::TempDir::new().unwrap();

    site.child("_redirects")
        .write_str(
            "# This is a comment\n\
             \n\
             /old-page /new-page.html 301\n\
             /external https://example.com/page\n\
             /broken /missing-page.html\n\
             /another /target.html",
        )
        .unwrap();

    site.child("new-page.html").touch().unwrap();
    site.child("target.html").touch().unwrap();

    site.child("index.html")
        .write_str("<a href='/old-page'>link</a>")
        .unwrap();

    let mut cmd = Command::cargo_bin("hyperlink").unwrap();
    cmd.current_dir(site.path()).arg(".");

    cmd.assert().failure().code(1).stdout(
        predicate::str::is_match(
            r#"^Reading files
Checking 4 links from 4 files \(4 documents\)
\./.*_redirects
  error: bad link /missing-page\.html

Found 1 bad links
"#,
        )
        .unwrap(),
    );

    site.close().unwrap();
}

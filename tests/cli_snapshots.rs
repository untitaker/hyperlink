use insta_cmd::{assert_cmd_snapshot, get_cargo_bin};
use std::process::Command;

fn cli() -> Command {
    Command::new(get_cargo_bin("hyperlink"))
}

#[test]
fn test_no_args() {
    assert_cmd_snapshot!(cli(), @r###"
    success: false
    exit_code: 1
    ----- stdout -----
    A command-line tool to find broken links in your static site.

    Usage: [-j=ARG] (COMMAND ... | [--check-anchors] [--sources=ARG] [--github-actions] [BASE-PATH])

    Available positional items:
        BASE-PATH             the static file path to check

    Available options:
        -V, --version         print version information and exit
        -j, --jobs=ARG        how many threads to use, default is to try and saturate CPU
            --check-anchors   whether to check for valid anchor references
            --sources=ARG     path to directory of markdown files to use for reporting errors
            --github-actions  enable specialized output for GitHub actions
        -h, --help            Prints help information

    Available commands:
        dump-paragraphs       Dump out internal data for markdown or html file.
        match-all-paragraphs  Attempt to match up all paragraphs from the HTML folder with the Markdown
                              folder and print
        dump-external-links   Dump out a list and count of _external_ links.  hyperlink does not check
                              external links,


    ----- stderr -----
    "###);
}

#[test]
fn test_dump_paragraphs_help() {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(r"hyperlink(\.exe)?", "[hyperlink bin]");
    let _guard = settings.bind_to_scope();

    assert_cmd_snapshot!(cli().arg("dump-paragraphs").arg("--help"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----
    Dump out internal data for markdown or html file.

    This is mostly useful to figure out why a source file is not properly matched up with its target
    html file.

    NOTE: This is a tool for debugging and development.

    Usage:
     
      vimdiff <([hyperlink bin] dump-paragraphs src/foo.md) <([hyperlink bin] dump-paragraphs public/foo.html)

    Each line on the left represents a Markdown paragraph. Each line on the right represents a HTML
    paragraph. If there are minor formatting differences in two lines that are supposed to match, you
    found the issue that needs fixing in `src/paragraph.rs`.

    Usage: [hyperlink bin] dump-paragraphs --file=ARG

    Available options:
            --file=ARG  markdown or html file
        -h, --help      Prints help information


    ----- stderr -----
    "###);
}

#[test]
fn test_version() {
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(r"hyperlink [.\d]+", "hyperlink [VERSION]");
    let _guard = settings.bind_to_scope();

    assert_cmd_snapshot!(cli().arg("-V"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----
    hyperlink [VERSION]

    ----- stderr -----
    "###);

    assert_cmd_snapshot!(cli().arg("--version"), @r###"
    success: true
    exit_code: 0
    ----- stdout -----
    hyperlink [VERSION]

    ----- stderr -----
    "###);
}

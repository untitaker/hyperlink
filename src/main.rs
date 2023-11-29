#![allow(clippy::manual_flatten)]
mod collector;
mod html;
mod markdown;
mod paragraph;

use std::cmp;
use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{anyhow, Context, Error};
use clap::Parser;
use jwalk::WalkDirGeneric;
use markdown::DocumentSource;
use rayon::prelude::*;

use collector::{BrokenLinkCollector, LocalLinksOnly, LinkCollector, UsedLinkCollector};
use html::{DefinedLink, Document, DocumentBuffers, Link};
use paragraph::{DebugParagraphWalker, NoopParagraphWalker, ParagraphHasher, ParagraphWalker};

use crate::html::is_external_url;

static MARKDOWN_FILES: &[&str] = &["md", "mdx"];
static HTML_FILES: &[&str] = &["htm", "html"];

#[derive(Parser)]
#[clap(about, version)]
struct Cli {
    /// The static file path to check.
    ///
    /// This will be assumed to be the root path of your server as well, so
    /// href="/foo" will resolve to that folder's subfolder foo.
    #[structopt(verbatim_doc_comment)]
    base_path: Option<PathBuf>,

    /// How many threads to use, default is to try and saturate CPU.
    #[clap(short = 'j', long = "jobs")]
    threads: Option<usize>,

    /// Whether to check for valid anchor references.
    #[clap(long = "check-anchors")]
    check_anchors: bool,

    /// Path to directory of markdown files to use for reporting errors.
    #[clap(long = "sources")]
    sources_path: Option<PathBuf>,

    /// Enable specialized output for GitHub actions.
    #[clap(long = "github-actions")]
    github_actions: bool,

    /// Utilities for development of hyperlink.
    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Parser)]
enum Subcommand {
    /// Dump out internal data for markdown or html file. This is mostly useful to figure out why
    /// a source file is not properly matched up with its target html file.
    ///
    /// NOTE: This is a tool for debugging and development.
    ///
    /// Usage:
    ///
    ///    vimdiff <(hyperlink dump-paragraphs src/foo.md) <(hyperlink dump-paragraphs public/foo.html)
    ///
    /// Each line on the left represents a Markdown paragraph. Each line on the right represents a
    /// HTML paragraph. If there are minor formatting differences in two lines that are supposed to
    /// match, you found the issue that needs fixing in `src/paragraph.rs`.
    ///
    /// There may also be entire lines missing from either side, in which case the logic for
    /// detecting paragraphs needs adjustment, either in `src/markdown.rs` or `src/html.rs`.
    ///
    /// Note that the output for HTML omits paragraphs that do not have links, while for Markdown
    /// all paragraphs are dumped.
    DumpParagraphs { file: PathBuf },

    /// Attempt to match up all paragraphs from the HTML folder with the Markdown folder and print
    /// stats. This can be used to determine whether the source matching is going to be any good.
    ///
    /// NOTE: This is a tool for debugging and development.
    MatchAllParagraphs {
        base_path: PathBuf,
        sources_path: PathBuf,
    },

    /// Dump out a list and count of _external_ links.  hyperlink does not check external links,
    /// but this subcommand can be used to get a summary of the external links that exist in your
    /// site.
    DumpExternalLinks {
        base_path: PathBuf,
    },
}

fn main() -> Result<(), Error> {
    let Cli {
        base_path,
        threads,
        check_anchors,
        sources_path,
        github_actions,
        subcommand,
    } = Cli::parse();

    rayon::ThreadPoolBuilder::new()
        // most of the work we do is kind of I/O bound. rayon assumes CPU-heavy workload. we could
        // look into tokio-uring at some point, but it seems like a hassle wrt ownership
        //
        // hyperlink seems to deadlock on less than 1 thread.
        .num_threads(cmp::max(2, threads.unwrap_or_else(|| 4 * num_cpus::get())))
        .build_global()
        .unwrap();

    match subcommand {
        Some(Subcommand::DumpParagraphs { file }) => {
            return dump_paragraphs(file);
        }
        Some(Subcommand::MatchAllParagraphs {
            base_path,
            sources_path,
        }) => {
            return match_all_paragraphs(base_path, sources_path);
        }
        Some(Subcommand::DumpExternalLinks { base_path }) => {
            return dump_external_links(base_path);
        },
        None => {}
    }

    let base_path = match base_path {
        Some(base_path) => base_path,
        None => {
            // Invalid invocation. Ultra hack to show help if no arguments are provided. Structopt
            // does not seem to have a functional way to require either an argument or a
            // subcommand. required_if etc don't actually work.
            let help_message = Cli::try_parse_from(["hyperlink", "--help"])
                .map(|_| ())
                .unwrap_err();
            help_message.print()?;
            process::exit(1);
        }
    };

    if sources_path.is_some() {
        check_links::<ParagraphHasher>(base_path, check_anchors, sources_path, github_actions)
    } else {
        check_links::<NoopParagraphWalker>(base_path, check_anchors, sources_path, github_actions)
    }
}

fn check_links<P: ParagraphWalker>(
    base_path: PathBuf,
    check_anchors: bool,
    sources_path: Option<PathBuf>,
    github_actions: bool,
) -> Result<(), Error>
where
    P::Paragraph: Copy + PartialEq,
{
    println!("Reading files");

    let html_result = extract_html_links::<LocalLinksOnly<BrokenLinkCollector<_>>, P>(&base_path, check_anchors)?;

    let used_links_len = html_result.collector.collector.used_links_count();
    println!(
        "Checking {} links from {} files ({} documents)",
        used_links_len, html_result.file_count, html_result.documents_count,
    );

    let mut bad_links_and_anchors = BTreeMap::new();
    let mut bad_links_count = 0;
    let mut bad_anchors_count = 0;

    let mut broken_links = html_result
        .collector
        .collector
        .get_broken_links(check_anchors)
        .peekable();

    let paragraps_to_sourcefile = if broken_links.peek().is_some() {
        if let Some(ref sources_path) = sources_path {
            println!("Found some broken links, reading source files");
            extract_markdown_paragraphs::<P>(sources_path)?
        } else {
            BTreeMap::new()
        }
    } else {
        BTreeMap::new()
    };

    for broken_link in broken_links {
        let mut had_sources = false;

        if broken_link.hard_404 {
            bad_links_count += 1;
        } else {
            bad_anchors_count += 1;
        }

        if let Some(ref paragraph) = broken_link.link.paragraph {
            if let Some(document_sources) = &paragraps_to_sourcefile.get(paragraph) {
                debug_assert!(!document_sources.is_empty());
                had_sources = true;

                for (source, lineno) in *document_sources {
                    let (bad_links, bad_anchors) = bad_links_and_anchors
                        .entry((!had_sources, source.path.clone()))
                        .or_insert_with(|| (BTreeSet::new(), BTreeSet::new()));

                    if broken_link.hard_404 {
                        bad_links
                    } else {
                        bad_anchors
                    }
                    .insert((Some(*lineno), broken_link.link.href.clone()));
                }
            }
        }

        if !had_sources {
            let (bad_links, bad_anchors) = bad_links_and_anchors
                .entry((!had_sources, broken_link.link.path))
                .or_insert_with(|| (BTreeSet::new(), BTreeSet::new()));

            if broken_link.hard_404 {
                bad_links
            } else {
                bad_anchors
            }
            .insert((None, broken_link.link.href));
        }
    }

    // _is_raw_file is an unused parameter that is only there to control iteration order over keys.
    // Sort markdown files to the start since otherwise the less valuable annotations on not
    // checked in files fill up the limit on annotations (tested manually, seems to be 10 right
    // now).
    for ((_is_raw_file, filepath), (bad_links, bad_anchors)) in bad_links_and_anchors {
        println!("{}", filepath.display());

        for (lineno, href) in &bad_links {
            print_href_error("error: bad link", href, *lineno);
        }

        for (lineno, href) in &bad_anchors {
            print_href_error("error: bad link", href, *lineno);
        }

        if github_actions {
            if !bad_links.is_empty() {
                print_github_actions_href_list("bad links", &filepath, &bad_links)?;
            }

            if !bad_anchors.is_empty() {
                print_github_actions_href_list("bad anchors", &filepath, &bad_anchors)?;
            }
        }

        println!();
    }

    println!("Found {bad_links_count} bad links");

    if check_anchors {
        println!("Found {bad_anchors_count} bad anchors");
    }

    // We're about to exit the program and leaking the memory is faster than running drop
    mem::forget(html_result);

    if bad_links_count > 0 {
        process::exit(1);
    }

    if bad_anchors_count > 0 {
        process::exit(2);
    }

    Ok(())
}

fn print_href_error(message: &'static str, href: &str, lineno: Option<usize>) {
    if let Some(lineno) = lineno {
        println!("  {message} /{href} at line {lineno}");
    } else {
        println!("  {message} /{href}");
    }
}

fn print_github_actions_href_list(
    message: &'static str,
    filepath: &Path,
    hrefs: &BTreeSet<(Option<usize>, String)>,
) -> Result<(), Error> {
    let mut prev_lineno = None;
    for (i, (lineno, href)) in hrefs.iter().enumerate() {
        if prev_lineno != *lineno || i == 0 {
            print!(
                "\n::error file={},line={}::{}:",
                filepath.canonicalize()?.display(),
                lineno.unwrap_or(1),
                message,
            );
        }
        prev_lineno = *lineno;

        // %0A -- escaped newline
        //
        // https://github.community/t/what-is-the-correct-character-escaping-for-workflow-command-values-e-g-echo-xxxx/118465/5
        print!("%0A  {}", href);
    }

    println!();

    Ok(())
}

fn dump_paragraphs(path: PathBuf) -> Result<(), Error> {
    let extension = match path.extension() {
        Some(x) => x,
        None => return Err(anyhow!("File has no extension, cannot determine type")),
    };

    let mut doc_buf = DocumentBuffers::default();

    let paragraphs: BTreeSet<_> = match extension.to_str() {
        Some(x) if MARKDOWN_FILES.contains(&x) => {
            let source = DocumentSource::new(path);
            source
                .paragraphs::<DebugParagraphWalker<ParagraphHasher>>()?
                .into_iter()
                .map(|(paragraph, lineno)| (paragraph, Some(lineno)))
                .collect()
        }
        Some(x) if HTML_FILES.contains(&x) => {
            let document = Document::new(Path::new(""), &path);
            document
                .links::<DebugParagraphWalker<ParagraphHasher>>(&mut doc_buf, false)?
                .filter_map(|link| Some((link.into_paragraph()?, None)))
                .collect()
        }
        _ => return Err(anyhow!("Unknown file extension")),
    };

    for (paragraph, lineno) in paragraphs {
        if let Some(lineno) = lineno {
            println!("{lineno}: {paragraph}");
        } else {
            println!("{paragraph}");
        }
    }

    Ok(())
}

fn dump_external_links(base_path: PathBuf) -> Result<(), Error> {
    println!("Reading files");
    let html_result =
        extract_html_links::<UsedLinkCollector<_>, NoopParagraphWalker>(&base_path, true)?;

    println!(
        "Checking {} links from {} files ({} documents)",
        html_result.collector.used_links.len(), html_result.file_count, html_result.documents_count,
    );

    let mut external_links = BTreeMap::new();
    let mut external_link_count: u32 = 0;

    let used_links = html_result
        .collector
        .used_links
        .iter()
        .peekable();


    for used_link in used_links {

        // check if the used link is external
        if is_external_url(used_link.href.as_str()) {
            external_link_count += 1;

            let external_links_at_path = external_links
                .entry(used_link.path.clone())
                .or_insert_with(|| BTreeSet::new());

            external_links_at_path.insert(&used_link.href);
        }
    }

    for (filepath, external_links_by_path) in external_links {
        println!("{}", filepath.display());

        for href in &external_links_by_path {
            println!("  info: external link {}", href);
        }

        println!();
    }

    println!("Found {} external links", external_link_count);

    mem::forget(html_result);

    Ok(())
}

struct HtmlResult<C> {
    collector: C,
    documents_count: usize,
    file_count: usize,
}

fn walk_files(
    base_path: &Path,
) -> impl ParallelIterator<Item = Result<jwalk::DirEntry<((), bool)>, jwalk::Error>> {
    WalkDirGeneric::<((), bool)>::new(base_path)
        .sort(true) // helps branch predictor (?)
        .skip_hidden(false)
        .process_read_dir(|_, _, _, children| {
            for dir_entry_result in children.iter_mut() {
                if let Ok(dir_entry) = dir_entry_result {
                    dir_entry.client_state = dir_entry.file_type().is_file();
                }
            }
        })
        .into_iter()
        .par_bridge()
        .filter_map(|entry_result| {
            if let Ok(entry) = entry_result {
                if let Some(err) = entry.read_children_error {
                    // https://github.com/Byron/jwalk/issues/40
                    return Some(Err(err));
                }

                if !entry.client_state {
                    return None;
                }
                Some(Ok(entry))
            } else {
                Some(entry_result)
            }
        })
}

fn extract_html_links<C: LinkCollector<P::Paragraph>, P: ParagraphWalker>(
    base_path: &Path,
    check_anchors: bool,
) -> Result<HtmlResult<C>, Error> {
    let result: Result<_, Error> = walk_files(base_path)
        .try_fold(
            || (DocumentBuffers::default(), C::new(), 0, 0),
            |(mut doc_buf, mut collector, mut documents_count, mut file_count), entry| {
                let entry = entry?;
                let path = entry.path();
                let document = Document::new(base_path, &path);

                collector.ingest(Link::Defines(DefinedLink {
                    href: document.href(),
                }));
                file_count += 1;

                if !document
                    .path
                    .extension()
                    .and_then(|extension| Some(HTML_FILES.contains(&extension.to_str()?)))
                    .unwrap_or(false)
                {
                    return Ok((doc_buf, collector, documents_count, file_count));
                }

                for link in document
                    .links::<P>(&mut doc_buf, check_anchors)
                    .with_context(|| format!("Failed to read file {}", document.path.display()))?
                {
                    collector.ingest(link);
                }

                doc_buf.reset();

                documents_count += 1;

                Ok((doc_buf, collector, documents_count, file_count))
            },
        )
        .map(|result| {
            result.map(|(_, collector, documents_count, file_count)| {
                (collector, documents_count, file_count)
            })
        })
        .try_reduce(
            || (C::new(), 0, 0),
            |(mut collector, mut documents_count, mut file_count),
             (collector2, documents_count2, file_count2)| {
                collector.merge(collector2);
                documents_count += documents_count2;
                file_count += file_count2;
                Ok((collector, documents_count, file_count))
            },
        );

    let (collector, documents_count, file_count) = result?;

    Ok(HtmlResult {
        collector,
        documents_count,
        file_count,
    })
}

type MarkdownResult<P> = BTreeMap<P, Vec<(DocumentSource, usize)>>;

fn extract_markdown_paragraphs<P: ParagraphWalker>(
    sources_path: &Path,
) -> Result<MarkdownResult<P::Paragraph>, Error> {
    let results: Vec<Result<_, Error>> = walk_files(sources_path)
        .try_fold(Vec::new, |mut paragraphs, entry| {
            let entry = entry?;
            let source = DocumentSource::new(entry.path());

            if !source
                .path
                .extension()
                .and_then(|extension| Some(MARKDOWN_FILES.contains(&extension.to_str()?)))
                .unwrap_or(false)
            {
                return Ok(paragraphs);
            }

            for paragraph_and_lineno in source
                .paragraphs::<P>()
                .with_context(|| format!("Failed to read file {}", source.path.display()))?
            {
                paragraphs.push((source.clone(), paragraph_and_lineno));
            }
            Ok(paragraphs)
        })
        .collect();

    let mut paragraps_to_sourcefile = BTreeMap::new();

    for result in results {
        for (source, (paragraph, lineno)) in result? {
            paragraps_to_sourcefile
                .entry(paragraph)
                .or_insert_with(Vec::new)
                .push((source.clone(), lineno));
        }
    }

    Ok(paragraps_to_sourcefile)
}

fn match_all_paragraphs(base_path: PathBuf, sources_path: PathBuf) -> Result<(), Error> {
    println!("Reading files");
    let html_result =
        extract_html_links::<UsedLinkCollector<_>, ParagraphHasher>(&base_path, true)?;

    println!("Reading source files");
    let paragraps_to_sourcefile = extract_markdown_paragraphs::<ParagraphHasher>(&sources_path)?;

    println!("Calculating");
    let mut total_links = 0;
    let mut link_no_paragraph = 0;
    let mut link_multiple_sources = 0;
    let mut link_no_source = 0;
    let mut link_single_source = 0;
    // We only care about HTML's used links because paragraph matching is exclusively for error
    // messages that point to the broken link.
    for link in &html_result.collector.used_links {
        total_links += 1;

        let paragraph = match link.paragraph {
            Some(ref p) => p,
            None => {
                link_no_paragraph += 1;
                continue;
            }
        };

        match paragraps_to_sourcefile.get(paragraph) {
            Some(sources) => {
                if sources.len() != 1 {
                    println!("multiple sources: {} in {}", link.href, link.path.display());
                    link_multiple_sources += 1;
                } else {
                    link_single_source += 1;
                }
            }
            None => {
                println!("no source: {} in {}", link.href, link.path.display());
                link_no_source += 1;
            }
        }
    }

    println!("{total_links} total links");
    println!("{link_no_paragraph} links outside of paragraphs");
    println!("{link_multiple_sources} links with multiple potential sources");
    println!("{link_no_source} links with no sources");
    println!("{link_single_source} links with one potential source (perfect match)");

    Ok(())
}

#[cfg(test)]
mod tests {
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
    fn test_no_args() {
        let mut cmd = Command::cargo_bin("hyperlink").unwrap();

        cmd.assert()
            .failure()
            .code(1)
            .stdout(predicate::str::contains(
                "\
USAGE:
    hyperlink [OPTIONS] [BASE_PATH] [SUBCOMMAND]\
",
            ));
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
}

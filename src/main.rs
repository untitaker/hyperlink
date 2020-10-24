mod html;
mod markdown;
mod paragraph;

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{anyhow, Context, Error};
use bumpalo::collections::Vec as BumpVec;
use bumpalo::Bump;
use jwalk::WalkDir;
use markdown::DocumentSource;
use rayon::prelude::*;
use structopt::StructOpt;
use thread_local::ThreadLocal;

use html::{DefinedLink, Document, Href, Link, UsedLink};
use paragraph::{DebugParagraphWalker, Paragraph, ParagraphHasher};

static MARKDOWN_FILES: &[&str] = &["md", "mdx"];
static HTML_FILES: &[&str] = &["htm", "html"];

#[derive(StructOpt)]
#[structopt(name = "hyperlink")]
struct Cli {
    /// The static file path to check.
    ///
    /// This will be assumed to be the root path of your server as well, so
    /// href="/foo" will resolve to that folder's subfolder foo.
    #[structopt(verbatim_doc_comment, required_if("subcommand", "None"))]
    base_path: Option<PathBuf>,

    /// How many threads to use, default is to try and saturate CPU.
    #[structopt(short = "j", long = "jobs")]
    threads: Option<usize>,

    /// Whether to check for valid anchor references.
    #[structopt(long = "check-anchors")]
    check_anchors: bool,

    /// Path to directory of markdown files to use for reporting errors.
    #[structopt(long = "sources")]
    sources_path: Option<PathBuf>,

    /// Enable specialized output for GitHub actions.
    #[structopt(long = "github-actions")]
    github_actions: bool,

    /// Utilities for development of hyperlink.
    #[structopt(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(StructOpt)]
enum Subcommand {
    /// Dump out internal data for markdown or html file. This is mostly useful to figure out why
    /// a source file is not properly matched up with its target html file.
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
}

fn main() -> Result<(), Error> {
    let Cli {
        base_path,
        threads,
        check_anchors,
        sources_path,
        github_actions,
        subcommand,
    } = Cli::from_args();

    if let Some(Subcommand::DumpParagraphs { file }) = subcommand {
        return dump_paragraphs(file);
    }

    let base_path = base_path.unwrap();

    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .unwrap();
    }

    let arenas = ThreadLocal::new();

    println!("Reading files");

    let html_result =
        extract_html_links(&arenas, &base_path, check_anchors, sources_path.is_some())?;

    let paragraps_to_sourcefile = if let Some(ref sources_path) = sources_path {
        println!("Discovering source files");
        extract_markdown_paragraphs(&arenas, sources_path)?
    } else {
        BTreeMap::new()
    };

    let used_links_len = html_result.used_links.len();
    println!(
        "Checking {} links from {} files ({} documents)",
        used_links_len, html_result.file_count, html_result.documents_count,
    );

    let mut bad_links_and_anchors = BTreeMap::new();
    let mut bad_links_count = 0;
    let mut bad_anchors_count = 0;

    for (&href, links) in &html_result.used_links {
        if !html_result.defined_links.contains(&href) {
            let hard_404 =
                !check_anchors || !html_result.defined_links.contains(&href.without_anchor());
            if hard_404 {
                bad_links_count += 1;
            } else {
                bad_anchors_count += 1;
            }

            for link in links {
                let mut had_sources = false;

                if let Some(ref paragraph) = link.paragraph {
                    if let Some(document_sources) = &paragraps_to_sourcefile.get(paragraph) {
                        debug_assert!(!document_sources.is_empty());
                        had_sources = true;

                        for source in *document_sources {
                            let (bad_links, bad_anchors) = bad_links_and_anchors
                                .entry((!had_sources, source.path.as_path()))
                                .or_insert_with(|| (BTreeSet::new(), BTreeSet::new()));

                            if hard_404 { bad_links } else { bad_anchors }.insert(href);
                        }
                    }
                }

                if !had_sources {
                    let (bad_links, bad_anchors) = bad_links_and_anchors
                        .entry((!had_sources, link.path))
                        .or_insert_with(|| (BTreeSet::new(), BTreeSet::new()));

                    if hard_404 { bad_links } else { bad_anchors }.insert(href);
                }
            }
        }
    }

    // _is_raw_file is an unused parameter that is only there to control iteration order over keys.
    // Sort markdown files to the start since otherwise the less valuable annotations on not
    // checked in files fill up the limit on annotations (tested manually, seems to be 10 right
    // now).
    for ((_is_raw_file, filepath), (bad_links, bad_anchors)) in bad_links_and_anchors {
        println!("{}", filepath.display());

        for href in &bad_links {
            println!("  error: bad link {}", href);
        }

        for href in &bad_anchors {
            println!("  warning: bad anchor {}", href);
        }

        if github_actions {
            if !bad_links.is_empty() {
                print!(
                    "::error file={}::bad links:",
                    filepath.canonicalize()?.display()
                );
                print_github_actions_href_list(&bad_links);
                println!();
            }

            if !bad_anchors.is_empty() {
                print!(
                    "::error file={}::bad anchors:",
                    filepath.canonicalize()?.display()
                );

                print_github_actions_href_list(&bad_anchors);
                println!();
            }
        }

        println!();
    }

    println!("Found {} bad links", bad_links_count);

    if check_anchors {
        println!("Found {} bad anchors", bad_anchors_count);
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

fn print_github_actions_href_list(hrefs: &BTreeSet<Href<'_>>) {
    for href in hrefs {
        // %0A -- escaped newline
        //
        // https://github.community/t/what-is-the-correct-character-escaping-for-workflow-command-values-e-g-echo-xxxx/118465/5
        print!("%0A  {}", href);
    }
}

fn dump_paragraphs(path: PathBuf) -> Result<(), Error> {
    let arena = Bump::new();

    let extension = match path.extension() {
        Some(x) => x,
        None => return Err(anyhow!("File has no extension, cannot determine type")),
    };

    let paragraphs: BTreeSet<_> = match extension.to_str() {
        Some(x) if MARKDOWN_FILES.contains(&x) => {
            let source = DocumentSource::new(path);
            source
                .paragraphs::<DebugParagraphWalker<ParagraphHasher>>()?
                .into_iter()
                .collect()
        }
        Some(x) if HTML_FILES.contains(&x) => {
            let document = Document::new(&arena, Path::new(""), &path);
            let mut links = Vec::new();
            document.links::<DebugParagraphWalker<ParagraphHasher>>(
                &arena,
                &mut Vec::new(),
                &mut links,
                false,
                true,
            )?;
            links
                .into_iter()
                .filter_map(|link| link.into_paragraph())
                .collect()
        }
        _ => return Err(anyhow!("Unknown file extension")),
    };

    for paragraph in paragraphs {
        println!("{}", paragraph);
    }

    Ok(())
}

struct HtmlResult<'a> {
    used_links: BTreeMap<Href<'a>, BumpVec<'a, UsedLink<'a, Paragraph>>>,
    defined_links: BTreeSet<Href<'a>>,
    documents_count: usize,
    file_count: usize,
}

fn extract_html_links<'a>(
    arenas: &'a ThreadLocal<Bump>,
    base_path: &Path,
    check_anchors: bool,
    get_paragraphs: bool,
) -> Result<HtmlResult<'a>, Error> {
    let entries = WalkDir::new(&base_path)
        .sort(true) // helps branch predictor (?)
        .into_iter()
        // XXX: cannot use par_bridge because of https://github.com/rayon-rs/rayon/issues/690
        .collect::<Vec<_>>();

    let extracted_links: Vec<Result<_, Error>> = entries
        .into_par_iter()
        .try_fold(
            // apparently can't use arena allocations here because that would make values !Send
            // also because quick-xml specifically wants std vec
            || (Vec::new(), Vec::new(), 0, 0),
            |(mut xml_buf, mut sink, mut documents_count, mut file_count), entry| {
                let entry = entry?;
                let metadata = entry.metadata()?;

                let file_type = metadata.file_type();

                if file_type.is_symlink() {
                    return Err(anyhow!(
                        "Found unsupported symlink at {}",
                        entry.path().display()
                    ));
                }

                if !file_type.is_file() {
                    return Ok((xml_buf, sink, documents_count, file_count));
                }

                let arena = arenas.get_or_default();
                let document = Document::new(&arena, &base_path, arena.alloc(entry.path()));

                sink.push(Link::Defines(DefinedLink {
                    href: document.href,
                }));
                file_count += 1;

                if !document
                    .path
                    .extension()
                    .and_then(|extension| Some(HTML_FILES.contains(&extension.to_str()?)))
                    .unwrap_or(false)
                {
                    return Ok((xml_buf, sink, documents_count, file_count));
                }

                document
                    .links::<ParagraphHasher>(
                        arena,
                        &mut xml_buf,
                        &mut sink,
                        check_anchors,
                        get_paragraphs,
                    )
                    .with_context(|| format!("Failed to read file {}", document.path.display()))?;

                xml_buf.clear();

                documents_count += 1;

                Ok((xml_buf, sink, documents_count, file_count))
            },
        )
        .collect();

    let mut defined_links = BTreeSet::new();
    let mut used_links = BTreeMap::new();
    let mut documents_count = 0;
    let mut file_count = 0;

    let main_arena = arenas.get_or_default();

    for result in extracted_links {
        let (_xml_buf, link_chunk, documents_count_chunk, file_count_chunk) = result?;
        documents_count += documents_count_chunk;
        file_count += file_count_chunk;

        for link in link_chunk {
            match link {
                Link::Uses(used_link) => {
                    used_links
                        .entry(used_link.href)
                        .or_insert_with(|| BumpVec::new_in(main_arena))
                        .push(used_link);
                }
                Link::Defines(defined_link) => {
                    defined_links.insert(defined_link.href);
                }
            }
        }
    }

    Ok(HtmlResult {
        defined_links,
        used_links,
        documents_count,
        file_count,
    })
}

type MarkdownResult<'a> = BTreeMap<Paragraph, BumpVec<'a, DocumentSource>>;

fn extract_markdown_paragraphs<'a>(
    arenas: &'a ThreadLocal<Bump>,
    sources_path: &Path,
) -> Result<MarkdownResult<'a>, Error> {
    let entries = WalkDir::new(sources_path)
        .sort(true) // helps branch predictor (?)
        .into_iter()
        // XXX: cannot use par_bridge because of https://github.com/rayon-rs/rayon/issues/690
        .collect::<Vec<_>>();

    let results: Vec<Result<_, Error>> = entries
        .into_par_iter()
        .try_fold(
            || (Vec::new()),
            |mut paragraphs, entry| {
                let entry = entry?;
                let metadata = entry.metadata()?;
                let file_type = metadata.file_type();

                if !file_type.is_file() {
                    return Ok(paragraphs);
                }

                let source = DocumentSource::new(entry.path());

                for paragraph in source
                    .paragraphs::<ParagraphHasher>()
                    .with_context(|| format!("Failed to read file {}", source.path.display()))?
                {
                    paragraphs.push((source.clone(), paragraph));
                }
                Ok(paragraphs)
            },
        )
        .collect();

    let mut paragraps_to_sourcefile = BTreeMap::new();
    let main_arena = arenas.get_or_default();

    for result in results {
        for (source, paragraph) in result? {
            paragraps_to_sourcefile
                .entry(paragraph)
                .or_insert_with(|| BumpVec::new_in(main_arena))
                .push(source.clone());
        }
    }

    Ok(paragraps_to_sourcefile)
}

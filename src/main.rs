mod html;
mod markdown;
mod paragraph;

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::PathBuf;
use std::process;

use anyhow::{anyhow, Context, Error};
use markdown::DocumentSource;
use rayon::prelude::*;
use structopt::StructOpt;
use walkdir::WalkDir;

use html::{Document, Link};

#[derive(StructOpt)]
#[structopt(name = "hyperlink")]
struct Cli {
    /// The static file path to check.
    ///
    /// This will be assumed to be the root path of your server as well, so
    /// href="/foo" will resolve to that folder's subfolder foo.
    #[structopt(verbatim_doc_comment)]
    base_path: PathBuf,

    /// How many threads to use, default is to try and saturate CPU.
    #[structopt(short = "j", long = "jobs")]
    threads: Option<usize>,

    /// Whether to check for valid anchor references.
    #[structopt(long = "check-anchors")]
    check_anchors: bool,

    /// Path to directory of markdown files to use for reporting errors.
    #[structopt(long = "sources")]
    sources_path: Option<PathBuf>,
}

fn main() -> Result<(), Error> {
    let Cli {
        base_path,
        threads,
        check_anchors,
        sources_path,
    } = Cli::from_args();

    let base_path = base_path.canonicalize()?;
    let sources_path = sources_path.map(|x| x.canonicalize()).transpose()?;

    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .unwrap();
    }

    let mut file_hrefs = BTreeSet::new();
    let mut documents = Vec::new();

    println!("Discovering files");

    for entry in WalkDir::new(&base_path) {
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
            continue;
        }

        let document = Document::new(&base_path, entry.path());

        if !file_hrefs.insert(document.href.clone()) {
            panic!("Found two files that would probably serve the same href. One of them is {}. Please file a bug with the output of 'find' on your folder.", entry.path().display());
        }

        if document
            .path
            .extension()
            .map_or(false, |extension| extension == "html" || extension == "htm")
        {
            documents.push(document);
        }
    }

    println!(
        "Checking {} out of {} files",
        documents.len(),
        file_hrefs.len()
    );

    let extracted_links: Result<_, Error> = documents
        .par_iter()
        .try_fold(
            || (BTreeMap::new(), BTreeSet::new()),
            |(mut used_links, mut defined_links), document| {
                document
                    .links(check_anchors, sources_path.is_some(), |link| match link {
                        Link::Uses(used_link) => {
                            used_links
                                .entry(used_link.href.clone())
                                .or_insert_with(Vec::new)
                                .push(used_link);
                        }
                        Link::Defines(defined_link) => {
                            // XXX: Use whole DefinedLink
                            defined_links.insert(defined_link.href);
                        }
                    })
                    .with_context(|| format!("Failed to read file {}", document.path.display()))?;

                Ok((used_links, defined_links))
            },
        )
        .try_reduce(
            || (BTreeMap::new(), BTreeSet::new()),
            |(mut used_links, mut defined_links), (used_links2, defined_links2)| {
                for (href, links) in used_links2 {
                    used_links
                        .entry(href)
                        .or_insert_with(Vec::new)
                        .extend(links);
                }

                defined_links.extend(defined_links2);
                Ok((used_links, defined_links))
            },
        );

    let (used_links, mut defined_links) = extracted_links?;
    defined_links.extend(file_hrefs);

    let mut paragraps_to_sourcefile = BTreeMap::new();

    if let Some(ref sources_path) = sources_path {
        println!("Discovering source files");

        let mut file_count = 0;
        let mut document_sources = Vec::new();

        for entry in WalkDir::new(sources_path) {
            file_count += 1;
            let entry = entry?;
            let metadata = entry.metadata()?;
            let file_type = metadata.file_type();

            if !file_type.is_file() {
                continue;
            }

            let source = DocumentSource::new(entry.path());

            if source
                .path
                .extension()
                .map_or(false, |extension| extension == "mdx" || extension == "md")
            {
                document_sources.push(source);
            }
        }

        println!(
            "Checking {} out of {} files in source folder",
            document_sources.len(),
            file_count
        );

        let results: Vec<_> = document_sources
            .par_iter()
            .map(|source| -> Result<_, Error> {
                // XXX: Inefficient
                let mut paragraphs = Vec::new();
                source
                    .paragraphs(|p| paragraphs.push(p))
                    .with_context(|| format!("Failed to read file {}", source.path.display()))?;
                Ok((source, paragraphs))
            })
            .collect();

        for result in results {
            let (source, paragraphs) = result?;
            for paragraph in paragraphs {
                paragraps_to_sourcefile
                    .entry(paragraph)
                    .or_insert_with(Vec::new)
                    .push(source.clone());
            }
        }
    }

    let used_links_len = used_links.len();
    let mut bad_links = 0;
    let mut bad_anchors = 0;

    for (href, links) in used_links {
        if !defined_links.contains(&href) {
            let hard_404 = !check_anchors || !defined_links.contains(&href.without_anchor());

            let expand_link_errors = |message| {
                'link: for link in &links {

                    if let Some(ref paragraph) = link.paragraph {
                        if let Some(document_sources) = &paragraps_to_sourcefile.get(paragraph) {
                            for source in *document_sources {
                                println!("{} {} at {} (found in {})", message, href, source.path.display(), link.path.display());
                            }
                            continue 'link;
                        }
                    }

                    println!("{} {} at {}", message, href, link.path.display());
                }
            };

            if hard_404 {
                bad_links += links.len();
                expand_link_errors("ERROR: Bad link");
            } else {
                bad_anchors += links.len();
                expand_link_errors("WARNING: Bad anchor");
            }
        }
    }

    println!("Checked {} links", used_links_len);
    println!("Checked {} files", documents.len());
    println!("Found {} bad links", bad_links);

    if check_anchors {
        println!("Found {} bad anchors", bad_anchors);
    }

    // We're about to exit the program and leaking the memory is faster than running drop
    mem::forget(defined_links);
    mem::forget(documents);

    if bad_links > 0 {
        process::exit(1);
    }

    if bad_anchors > 0 {
        process::exit(2);
    }

    Ok(())
}

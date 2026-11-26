//! The download queue: one download at a time into the downloads folder, with pause and
//! resume (HTTP ranges) and the `.meta` MO2 writes next to each archive.
//!
//! As MO2's `DownloadManager` does, a running download is `<file>.unfinished` with its
//! `<file>.unfinished.meta` (`paused=true` while paused); when it completes both are renamed
//! to `<file>` and `<file>.meta`. Resuming asks for `Range: bytes=<size>-`; a server that
//! answers 200 instead of 206 restarts the file from zero.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use strata_ini::{Document, Value};

use crate::{Error, Result};

/// What the `.meta` of a download records (`DownloadManager`'s keys).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadInfo {
    pub game_name: String,
    pub mod_id: i64,
    pub file_id: i64,
    pub name: String,
    pub description: String,
    pub mod_name: String,
    pub version: String,
    pub newest_version: String,
    pub file_category: i64,
    pub category: i64,
    pub repository: String,
}

/// One download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub url: String,
    /// The archive's name in the downloads folder.
    pub file_name: String,
    pub info: DownloadInfo,
    pub state: State,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Paused,
    Done,
    Failed,
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    /// The archive is complete at this path.
    Done(PathBuf),
    /// Paused after this many bytes; `<file>.unfinished` stays.
    Paused(u64),
}

fn unfinished(dir: &Path, file: &str) -> PathBuf {
    dir.join(format!("{file}.unfinished"))
}

fn meta_of(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".meta");
    s.into()
}

fn write_meta(path: &Path, job: &Job, paused: bool) -> Result<()> {
    let meta = meta_of(path);
    let mut doc = match std::fs::read(&meta) {
        Ok(data) => Document::parse(&data),
        Err(_) => Document::parse(b"[General]\n"),
    };
    let i = &job.info;
    let s = |v: &str| Value::String(v.to_owned());
    for (key, value) in [
        ("gameName", s(&i.game_name)),
        ("modID", s(&i.mod_id.to_string())),
        ("fileID", s(&i.file_id.to_string())),
        ("url", s(&job.url)),
        ("name", s(&i.name)),
        ("description", s(&i.description)),
        ("modName", s(&i.mod_name)),
        ("version", s(&i.version)),
        ("newestVersion", s(&i.newest_version)),
        ("fileCategory", s(&i.file_category.to_string())),
        ("category", s(&i.category.to_string())),
        ("repository", s(&i.repository)),
        ("installed", s("false")),
        ("uninstalled", s("false")),
        ("paused", s(if paused { "true" } else { "false" })),
        ("removed", s("false")),
    ] {
        doc.set("", key, &value);
    }
    crate::write_meta_file(&meta, &doc.to_bytes())
}

/// The URL as HTTP wants it: Nexus' download links carry raw spaces (and may carry other
/// characters a URI cannot hold), which Qt's `QUrl` encodes on its own. Bytes outside the
/// URI characters become `%XX`; existing escapes stay.
pub fn request_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b);
        if keep {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Downloads `job` into `dir` (resuming a `.unfinished` file). `progress(done, total)` is
/// called as bytes arrive; setting `pause` stops after the current chunk.
pub fn download(
    agent: &ureq::Agent,
    job: &Job,
    dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
    pause: &AtomicBool,
) -> Result<Finished> {
    let target = dir.join(&job.file_name);
    if target.exists() {
        return Err(Error::Exists(target));
    }
    let part = unfinished(dir, &job.file_name);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    write_meta(&part, job, false)?;

    let mut request = agent.get(&request_url(&job.url));
    if have > 0 {
        request = request.header("Range", &format!("bytes={have}-"));
    }
    let mut reply = request.call().map_err(|e| Error::Network(e.to_string()))?;
    let status = reply.status().as_u16();
    let (mut done, append) = match status {
        206 => (have, true),
        200 => (0, false),
        // the range is past the end: the file is complete
        416 if have > 0 => (have, true),
        s => return Err(Error::Api { status: s, message: format!("download of {} failed", job.file_name) }),
    };
    let length: Option<u64> = reply.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let total = length.map(|l| l + done);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .map_err(|e| Error::Io(part.clone(), e))?;
    if status != 416 {
        let mut reader = reply.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if pause.load(Ordering::Relaxed) {
                file.flush().map_err(|e| Error::Io(part.clone(), e))?;
                write_meta(&part, job, true)?;
                return Ok(Finished::Paused(done));
            }
            let n = reader.read(&mut buf).map_err(|e| Error::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| Error::Io(part.clone(), e))?;
            done += n as u64;
            progress(done, total);
        }
    }
    file.flush().map_err(|e| Error::Io(part.clone(), e))?;
    drop(file);
    if let Some(total) = total
        && done < total
    {
        write_meta(&part, job, true)?;
        return Err(Error::Network(format!("the connection ended after {done} of {total} bytes; resume to continue")));
    }
    std::fs::rename(meta_of(&part), meta_of(&target)).map_err(|e| Error::Io(part.clone(), e))?;
    std::fs::rename(&part, &target).map_err(|e| Error::Io(part.clone(), e))?;
    Ok(Finished::Done(target))
}

impl Job {
    /// A paused download as its `<file>.unfinished.meta` describes it (the URL and the info
    /// written when it started), to resume it after a restart.
    pub fn from_unfinished(dir: &Path, file_name: &str) -> Result<Job> {
        let meta = meta_of(&unfinished(dir, file_name));
        let data = std::fs::read(&meta).map_err(|e| Error::Io(meta.clone(), e))?;
        let doc = Document::parse(&data);
        let text = |k: &str| doc.string("", k).unwrap_or_default();
        let int = |k: &str| text(k).trim().parse::<i64>().unwrap_or(0);
        let url = text("url");
        if url.is_empty() {
            return Err(Error::Format(format!("{} has no url to resume from", meta.display())));
        }
        Ok(Job {
            url,
            file_name: file_name.to_owned(),
            info: DownloadInfo {
                game_name: text("gameName"),
                mod_id: int("modID"),
                file_id: int("fileID"),
                name: text("name"),
                description: text("description"),
                mod_name: text("modName"),
                version: text("version"),
                newest_version: text("newestVersion"),
                file_category: int("fileCategory"),
                category: int("category"),
                repository: text("repository"),
            },
            state: State::Paused,
        })
    }
}

/// An agent for [`download`]: HTTP errors are replies, not failures.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

/// Downloads one after the other.
pub struct Queue {
    pub dir: PathBuf,
    pub jobs: VecDeque<Job>,
    agent: ureq::Agent,
}

impl Queue {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, jobs: VecDeque::new(), agent: agent() }
    }

    pub fn push(&mut self, url: &str, file_name: &str, info: DownloadInfo) {
        self.jobs.push_back(Job { url: url.to_owned(), file_name: file_name.to_owned(), info, state: State::Pending });
    }

    /// Makes a paused or failed job pending again.
    pub fn resume(&mut self, index: usize) {
        if let Some(job) = self.jobs.get_mut(index)
            && matches!(job.state, State::Paused | State::Failed)
        {
            job.state = State::Pending;
        }
    }

    /// Runs the next pending job. Returns its index and how it ended, `None` when nothing is
    /// pending.
    pub fn run_next(
        &mut self,
        progress: &mut dyn FnMut(usize, u64, Option<u64>),
        pause: &AtomicBool,
    ) -> Option<(usize, Result<Finished>)> {
        let index = self.jobs.iter().position(|j| j.state == State::Pending)?;
        let job = self.jobs[index].clone();
        let result = download(&self.agent, &job, &self.dir, &mut |d, t| progress(index, d, t), pause);
        self.jobs[index].state = match &result {
            Ok(Finished::Done(_)) => State::Done,
            Ok(Finished::Paused(_)) => State::Paused,
            Err(_) => State::Failed,
        };
        pause.store(false, Ordering::Relaxed);
        Some((index, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A tiny HTTP server for one file, honouring `Range: bytes=N-`; records the ranges.
    fn serve(data: Vec<u8>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.7z", listener.local_addr().unwrap());
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let seen = ranges.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse::<usize>().ok();
                    }
                }
                seen.lock().unwrap().push(range.map_or("none".into(), |r| r.to_string()));
                let mut out = stream;
                let start = range.unwrap_or(0);
                let body = &data[start..];
                let head = if range.is_some() {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                };
                let _ = out.write_all(head.as_bytes());
                for chunk in body.chunks(8192) {
                    if out.write_all(chunk).is_err() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        });
        (url, ranges)
    }

    #[test]
    fn pause_then_resume_with_a_range() {
        let data: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
        let (url, ranges) = serve(data.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut q = Queue::new(dir.path().to_owned());
        let info = DownloadInfo { game_name: "skyrimse".into(), mod_id: 12604, file_id: 35407, name: "SkyUI".into(), repository: "Nexus".into(), ..DownloadInfo::default() };
        q.push(&url, "SkyUI-12604.7z", info);
        let pause = AtomicBool::new(false);
        let (_, r) = q
            .run_next(
                &mut |_, done, _| {
                    if done > 100_000 {
                        pause.store(true, Ordering::Relaxed);
                    }
                },
                &pause,
            )
            .unwrap();
        let Finished::Paused(at) = r.unwrap() else { panic!("not paused") };
        assert!(at > 100_000 && at < 400_000);
        let part_meta = std::fs::read_to_string(dir.path().join("SkyUI-12604.7z.unfinished.meta")).unwrap();
        assert!(part_meta.contains("paused=true") && part_meta.contains("modID=12604"));
        assert!(q.run_next(&mut |_, _, _| {}, &pause).is_none(), "nothing pending while paused");

        // after a restart the queue is gone: the .unfinished.meta tells how to go on
        let job = Job::from_unfinished(dir.path(), "SkyUI-12604.7z").unwrap();
        assert_eq!((job.url.as_str(), job.info.mod_id, job.info.name.as_str()), (url.as_str(), 12604, "SkyUI"));
        let mut q = Queue::new(dir.path().to_owned());
        q.jobs.push_back(job);
        q.resume(0);
        let (_, r) = q.run_next(&mut |_, _, _| {}, &pause).unwrap();
        let Finished::Done(path) = r.unwrap() else { panic!("not done") };
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert!(!dir.path().join("SkyUI-12604.7z.unfinished").exists());
        let meta = std::fs::read_to_string(dir.path().join("SkyUI-12604.7z.meta")).unwrap();
        assert!(meta.contains("paused=false") && meta.contains(&format!("url={url}")) && meta.contains("installed=false"));
        assert_eq!(*ranges.lock().unwrap(), vec!["none".to_owned(), at.to_string()]);
        assert_eq!(q.jobs[0].state, State::Done);
    }

    #[test]
    fn an_existing_archive_is_not_overwritten() {
        let (url, _) = serve(b"abc".to_vec());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.7z"), "old").unwrap();
        let job = Job { url, file_name: "a.7z".into(), info: DownloadInfo::default(), state: State::Pending };
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        assert!(matches!(download(&agent, &job, dir.path(), &mut |_, _| {}, &AtomicBool::new(false)), Err(Error::Exists(_))));
    }

    #[test]
    fn urls_with_spaces_are_encoded() {
        assert_eq!(
            request_url("https://cf-files.nexusmods.com/cdn/1704/106172/Skyrim Misc Fixes-1.zip?md5=a%2Bb&expires=1"),
            "https://cf-files.nexusmods.com/cdn/1704/106172/Skyrim%20Misc%20Fixes-1.zip?md5=a%2Bb&expires=1"
        );
        assert_eq!(request_url("http://h/Ünï.7z"), "http://h/%C3%9Cn%C3%AF.7z");
    }
}

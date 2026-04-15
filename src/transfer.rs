//! SFTP transfers between [[servers]] (local or remote), preserving mode and timestamps like `rsync -a`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ssh2::{FileStat, OpenFlags, OpenType, Session, Sftp};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::{ConfigBundle, Server, TransferSpec};
use crate::error::{GraphRunError, Result};

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFLNK: u32 = 0o120_000;

fn perm_kind(perm: Option<u32>) -> Option<u32> {
    perm.map(|p| p & S_IFMT)
}

fn is_dir(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFDIR)
}

fn is_reg(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFREG)
}

fn is_lnk(st: &FileStat) -> bool {
    perm_kind(st.perm) == Some(S_IFLNK)
}

fn mode_for_file(st: &FileStat) -> i32 {
    (st.perm.unwrap_or(0o644) & 0o7777) as i32
}

fn mode_for_mkdir(st: &FileStat) -> i32 {
    (st.perm.unwrap_or(0o755) & 0o7777) as i32
}

fn ssh_err(e: ssh2::Error) -> GraphRunError {
    GraphRunError::msg(format!("SSH/SFTP: {e}"))
}

pub(crate) fn ssh_connect_session(server: &Server, timeout_ms: u32) -> Result<Session> {
    if server.kind != "remote" {
        return Err(GraphRunError::msg(format!(
            "transfer: server {} is not remote (kind={})",
            server.id, server.kind
        )));
    }
    let host = server
        .host
        .as_deref()
        .ok_or_else(|| GraphRunError::msg(format!("transfer: remote server {} has no host", server.id)))?;
    let user = server
        .user
        .as_deref()
        .ok_or_else(|| GraphRunError::msg(format!("transfer: remote server {} has no user", server.id)))?;
    let port = server.port.unwrap_or(22);
    let addr = format!("{host}:{port}");
    let tcp = TcpStream::connect(&addr).map_err(|e| GraphRunError::msg(format!("SSH connect {addr}: {e}")))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_millis(timeout_ms as u64)));
    let _ = tcp.set_write_timeout(Some(Duration::from_millis(timeout_ms as u64)));

    let mut sess = Session::new().map_err(ssh_err)?;
    sess.set_tcp_stream(tcp);
    sess.set_blocking(true);
    sess.set_timeout(timeout_ms);
    sess.handshake().map_err(ssh_err)?;

    if let Some(pw) = server.resolved_password() {
        if !pw.is_empty() {
            sess.userauth_password(user, &pw).map_err(ssh_err)?;
        }
    }
    if !sess.authenticated() {
        sess.userauth_agent(user).map_err(ssh_err)?;
    }
    if !sess.authenticated() {
        return Err(GraphRunError::msg(format!(
            "SSH authentication failed for server {} ({}@{})",
            server.id, user, host
        )));
    }
    Ok(sess)
}

/// Replace `$GRAPH_RUN_WORKSPACE` / `$GRAPH_RUN_TMP` when a workspace root is available.
fn expand_graph_run_tokens(path: &str, workspace_root: Option<&Path>) -> Result<String> {
    let mut s = path.to_string();
    if s.contains("$GRAPH_RUN_WORKSPACE") {
        let Some(root) = workspace_root else {
            return Err(GraphRunError::msg(
                "transfer path contains $GRAPH_RUN_WORKSPACE but no workspace directory is configured for this run",
            ));
        };
        s = s.replace("$GRAPH_RUN_WORKSPACE", &root.to_string_lossy());
    }
    if s.contains("$GRAPH_RUN_TMP") {
        let Some(root) = workspace_root else {
            return Err(GraphRunError::msg(
                "transfer path contains $GRAPH_RUN_TMP but no workspace directory is configured for this run",
            ));
        };
        s = s.replace("$GRAPH_RUN_TMP", &root.join("tmp").to_string_lossy());
    }
    Ok(s)
}

fn expand_local_home(path: &str) -> Result<String> {
    let mut s = path.to_string();
    if s.contains("$HOME") {
        let h = std::env::var("HOME").map_err(|_| {
            GraphRunError::msg("transfer path contains $HOME but HOME is not set in graph_run's environment")
        })?;
        s = s.replace("$HOME", &h);
    }
    Ok(s)
}

/// Paths used on the graph_run host (local SFTP or local disk).
fn expand_local_transfer_path(path: &str, workspace_root: Option<&Path>) -> Result<String> {
    let s = expand_graph_run_tokens(path, workspace_root)?;
    expand_local_home(&s)
}

fn read_remote_home(sess: &Session) -> Result<String> {
    let mut channel = sess.channel_session().map_err(ssh_err)?;
    channel
        .exec("sh -c 'printf %s \"$HOME\"'")
        .map_err(ssh_err)?;
    let mut out = Vec::new();
    io::copy(&mut channel, &mut out).map_err(|e| GraphRunError::msg(format!("read remote $HOME: {e}")))?;
    channel.wait_close().map_err(ssh_err)?;
    let code = channel.exit_status().map_err(ssh_err)?;
    if code != 0 {
        return Err(GraphRunError::msg(format!(
            "remote shell probing HOME exited with status {code}"
        )));
    }
    let s = String::from_utf8_lossy(&out).trim().to_string();
    if s.is_empty() {
        return Err(GraphRunError::msg("remote HOME resolved to an empty string"));
    }
    Ok(s)
}

/// Paths used on a remote SSH/SFTP server. `$HOME` is resolved with one `exec` on `sess` before SFTP.
fn expand_remote_transfer_path(path: &str, sess: &Session, workspace_root: Option<&Path>) -> Result<String> {
    let mut s = expand_graph_run_tokens(path, workspace_root)?;
    if s.contains("$HOME") {
        let home = read_remote_home(sess)?;
        s = s.replace("$HOME", &home);
    }
    Ok(s)
}

/// SFTP `readdir` may return each name as a basename or as a **full absolute path**. `Path::join`
/// discards the left path when the right is absolute, which would place files under `/` on the
/// wrong machine; use this for the remote side of `parent` + `name`.
fn remote_sftp_entry_path(parent: &Path, name: &Path) -> PathBuf {
    if name.is_absolute() {
        name.to_path_buf()
    } else {
        parent.join(name)
    }
}

/// Map a path under `src_root` on the source side to the same relative path under `dst_root`
/// (used for remote→local and remote→remote when `readdir` returns absolute names).
fn path_under_mirror_root(src_root: &Path, dst_root: &Path, path_under_src: &Path) -> Result<PathBuf> {
    let rel = path_under_src.strip_prefix(src_root).map_err(|_| {
        GraphRunError::msg(format!(
            "transfer: path {:?} is not under source root {:?}",
            path_under_src.display(),
            src_root.display()
        ))
    })?;
    if rel.as_os_str().is_empty() {
        return Ok(dst_root.to_path_buf());
    }
    Ok(dst_root.join(rel))
}

/// Debug: TOML paths vs expanded paths used for SFTP / local mkdir (needs **`-vvv`** / `RUST_LOG=graph_run=debug`).
fn log_transfer_paths(mode: &str, spec: &TransferSpec, expanded_src: &str, expanded_dst: &str) {
    log::debug!(
        target: "graph_run",
        "transfer expand ({mode}): source TOML={:?} -> {:?}; dest TOML={:?} -> {:?}; source_ends_with_slash={}",
        spec.source_path,
        expanded_src,
        spec.dest_path,
        expanded_dst,
        spec.source_path.ends_with('/'),
    );
}

fn mkdir_p_remote(sftp: &Sftp, path: &Path, default_mode: i32) -> Result<()> {
    if path.as_os_str().is_empty() || path == Path::new("/") {
        return Ok(());
    }
    match sftp.stat(path) {
        Ok(st) => {
            if is_dir(&st) {
                return Ok(());
            }
            return Err(GraphRunError::msg(format!(
                "mkdir_p: {} exists and is not a directory",
                path.display()
            )));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        mkdir_p_remote(sftp, parent, default_mode)?;
    }
    sftp.mkdir(path, default_mode).map_err(ssh_err)?;
    Ok(())
}

fn mkdir_p_local(path: &Path, default_mode: u32) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| GraphRunError::msg(format!("mkdir {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::Permissions::from_mode(default_mode & 0o777);
        let _ = fs::set_permissions(path, mode);
    }
    Ok(())
}

fn setstat_remote(sftp: &Sftp, path: &Path, st: &FileStat) -> Result<()> {
    sftp
        .setstat(
            path,
            FileStat {
                size: None,
                uid: st.uid,
                gid: st.gid,
                perm: st.perm,
                atime: st.atime,
                mtime: st.mtime,
            },
        )
        .map_err(ssh_err)
}

fn copy_remote_file_to_remote(sa: &Sftp, sb: &Sftp, from: &Path, to: &Path, st: &FileStat) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sb, parent, 0o755)?;
    }
    let mut rf = sa
        .open_mode(from, OpenFlags::READ, 0, OpenType::File)
        .map_err(ssh_err)?;
    let mode = mode_for_file(st);
    let mut wf = sb
        .open_mode(
            to,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
            mode,
            OpenType::File,
        )
        .map_err(ssh_err)?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("sftp copy: {e}")))?;
    wf.setstat(FileStat {
        size: st.size,
        uid: st.uid,
        gid: st.gid,
        perm: st.perm,
        atime: st.atime,
        mtime: st.mtime,
    })
    .map_err(ssh_err)?;
    Ok(())
}

fn copy_remote_symlink_to_remote(sa: &Sftp, sb: &Sftp, from: &Path, to: &Path) -> Result<()> {
    let target = sa.readlink(from).map_err(ssh_err)?;
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sb, parent, 0o755)?;
    }
    sb.symlink(&target, to).map_err(ssh_err)
}

fn copy_remote_tree_inner(
    sa: &Sftp,
    sb: &Sftp,
    src_curr: &Path,
    dst_curr: &Path,
    src_root: &Path,
    dst_root: &Path,
    src_contents_only: bool,
) -> Result<()> {
    let st = sa.lstat(src_curr).map_err(ssh_err)?;
    if is_lnk(&st) {
        return copy_remote_symlink_to_remote(sa, sb, src_curr, dst_curr);
    }
    if is_reg(&st) {
        return copy_remote_file_to_remote(sa, sb, src_curr, dst_curr, &st);
    }
    if !is_dir(&st) {
        return Err(GraphRunError::msg(format!(
            "unsupported source file type (not file/dir/symlink): {}",
            src_curr.display()
        )));
    }

    if src_contents_only {
        mkdir_p_remote(sb, dst_curr, mode_for_mkdir(&st))?;
        setstat_remote(sb, dst_curr, &st)?;
        for (name, _) in sa.readdir(src_curr).map_err(ssh_err)? {
            if name == Path::new(".") || name == Path::new("..") {
                continue;
            }
            let child_src = remote_sftp_entry_path(src_curr, &name);
            let child_dst = path_under_mirror_root(src_root, dst_root, &child_src)?;
            copy_remote_tree_inner(sa, sb, &child_src, &child_dst, src_root, dst_root, false)?;
        }
        return Ok(());
    }

    mkdir_p_remote(sb, dst_curr, mode_for_mkdir(&st))?;
    setstat_remote(sb, dst_curr, &st)?;
    for (name, _) in sa.readdir(src_curr).map_err(ssh_err)? {
        if name == Path::new(".") || name == Path::new("..") {
            continue;
        }
        let child_src = remote_sftp_entry_path(src_curr, &name);
        let child_dst = path_under_mirror_root(src_root, dst_root, &child_src)?;
        copy_remote_tree_inner(sa, sb, &child_src, &child_dst, src_root, dst_root, false)?;
    }
    Ok(())
}

fn copy_remote_to_remote(
    src_srv: &Server,
    dst_srv: &Server,
    spec: &TransferSpec,
    timeout_ms: u32,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let sa = ssh_connect_session(src_srv, timeout_ms)?;
    let src_s = expand_remote_transfer_path(&spec.source_path, &sa, workspace_root)?;
    let sb = ssh_connect_session(dst_srv, timeout_ms)?;
    let dst_s = expand_remote_transfer_path(&spec.dest_path, &sb, workspace_root)?;
    log_transfer_paths("remote->remote", spec, &src_s, &dst_s);
    let sftp_a = sa.sftp().map_err(ssh_err)?;
    let sftp_b = sb.sftp().map_err(ssh_err)?;
    let src = Path::new(&src_s);
    let dst = Path::new(&dst_s);
    let src_slash = spec.source_path.ends_with('/');
    copy_remote_tree_inner(&sftp_a, &sftp_b, src, dst, src, dst, src_slash)
}

fn copy_local_file_to_remote(sftp: &Sftp, from: &Path, to: &Path, meta: &fs::Metadata) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_remote(sftp, parent, 0o755)?;
    }
    let mut rf = fs::File::open(from).map_err(|e| GraphRunError::msg(format!("open {}: {e}", from.display())))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        (meta.permissions().mode() & 0o7777) as i32
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    let mut wf = sftp
        .open_mode(
            to,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
            mode,
            OpenType::File,
        )
        .map_err(ssh_err)?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
    let atime = meta.accessed().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    wf.setstat(FileStat {
        size: Some(meta.len()),
        uid: None,
        gid: None,
        perm: None,
        atime,
        mtime,
    })
    .map_err(ssh_err)?;
    Ok(())
}

fn copy_local_tree_to_remote(sftp: &Sftp, src: &Path, dst: &Path, src_contents_only: bool) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .map_err(|e| GraphRunError::msg(format!("stat {}: {e}", src.display())))?;
    if meta.is_symlink() {
        let target = fs::read_link(src).map_err(|e| GraphRunError::msg(format!("readlink: {e}")))?;
        if let Some(parent) = dst.parent() {
            mkdir_p_remote(sftp, parent, 0o755)?;
        }
        return sftp.symlink(&target, dst).map_err(ssh_err);
    }
    if meta.is_file() {
        return copy_local_file_to_remote(sftp, src, dst, &meta);
    }
    if !meta.is_dir() {
        return Err(GraphRunError::msg(format!("unsupported local type: {}", src.display())));
    }

    if src_contents_only {
        mkdir_p_remote(sftp, dst, 0o755)?;
        for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
            let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
            copy_local_tree_to_remote(sftp, &ent.path(), &dst.join(ent.file_name()), false)?;
        }
        return Ok(());
    }

    mkdir_p_remote(sftp, dst, 0o755)?;
    for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
        let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
        copy_local_tree_to_remote(sftp, &ent.path(), &dst.join(ent.file_name()), false)?;
    }
    Ok(())
}

fn copy_local_to_remote(
    src_srv: &Server,
    dst_srv: &Server,
    spec: &TransferSpec,
    timeout_ms: u32,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let _ = src_srv;
    let src_s = expand_local_transfer_path(&spec.source_path, workspace_root)?;
    let sess = ssh_connect_session(dst_srv, timeout_ms)?;
    let dst_s = expand_remote_transfer_path(&spec.dest_path, &sess, workspace_root)?;
    log_transfer_paths("local->remote", spec, &src_s, &dst_s);
    let sftp = sess.sftp().map_err(ssh_err)?;
    let src = Path::new(&src_s);
    let dst = Path::new(&dst_s);
    copy_local_tree_to_remote(&sftp, src, dst, spec.source_path.ends_with('/'))
}

fn copy_remote_file_to_local(sftp: &Sftp, from: &Path, to: &Path, st: &FileStat) -> Result<()> {
    if let Some(parent) = to.parent() {
        mkdir_p_local(parent, 0o755)?;
    }
    let mut rf = sftp
        .open_mode(from, OpenFlags::READ, 0, OpenType::File)
        .map_err(ssh_err)?;
    let mut wf = fs::File::create(to).map_err(|e| GraphRunError::msg(format!("create {}: {e}", to.display())))?;
    io::copy(&mut rf, &mut wf).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(p) = st.perm {
            let _ = fs::set_permissions(to, fs::Permissions::from_mode(p & 0o7777));
        }
    }
    if let Some(m) = st.mtime {
        let ft = filetime::FileTime::from_unix_time(m as i64, 0);
        filetime::set_file_mtime(to, ft).map_err(|e| GraphRunError::msg(format!("set mtime: {e}")))?;
    }
    Ok(())
}

fn copy_remote_tree_to_local(
    sftp: &Sftp,
    src_curr: &Path,
    dst_curr: &Path,
    src_root: &Path,
    dst_root: &Path,
    src_contents_only: bool,
) -> Result<()> {
    let st = sftp.lstat(src_curr).map_err(ssh_err)?;
    if is_lnk(&st) {
        let target = sftp.readlink(src_curr).map_err(ssh_err)?;
        if let Some(parent) = dst_curr.parent() {
            mkdir_p_local(parent, 0o755)?;
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, dst_curr).map_err(|e| GraphRunError::msg(format!("symlink: {e}")))?;
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err(GraphRunError::msg("symlink copy to local is only supported on Unix"));
    }
    if is_reg(&st) {
        return copy_remote_file_to_local(sftp, src_curr, dst_curr, &st);
    }
    if !is_dir(&st) {
        return Err(GraphRunError::msg("unsupported remote file type"));
    }

    if src_contents_only {
        mkdir_p_local(dst_curr, 0o755)?;
        for (name, _) in sftp.readdir(src_curr).map_err(ssh_err)? {
            if name == Path::new(".") || name == Path::new("..") {
                continue;
            }
            let child_src = remote_sftp_entry_path(src_curr, &name);
            let child_dst = path_under_mirror_root(src_root, dst_root, &child_src)?;
            copy_remote_tree_to_local(sftp, &child_src, &child_dst, src_root, dst_root, false)?;
        }
        return Ok(());
    }

    mkdir_p_local(dst_curr, mode_for_mkdir(&st) as u32)?;
    for (name, _) in sftp.readdir(src_curr).map_err(ssh_err)? {
        if name == Path::new(".") || name == Path::new("..") {
            continue;
        }
        let child_src = remote_sftp_entry_path(src_curr, &name);
        let child_dst = path_under_mirror_root(src_root, dst_root, &child_src)?;
        copy_remote_tree_to_local(sftp, &child_src, &child_dst, src_root, dst_root, false)?;
    }
    Ok(())
}

fn copy_remote_to_local(
    src_srv: &Server,
    dst_srv: &Server,
    spec: &TransferSpec,
    timeout_ms: u32,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let _ = dst_srv;
    let sess = ssh_connect_session(src_srv, timeout_ms)?;
    let src_s = expand_remote_transfer_path(&spec.source_path, &sess, workspace_root)?;
    let dst_s = expand_local_transfer_path(&spec.dest_path, workspace_root)?;
    log_transfer_paths("remote->local", spec, &src_s, &dst_s);
    let sftp = sess.sftp().map_err(ssh_err)?;
    let src = Path::new(&src_s);
    let dst = Path::new(&dst_s);
    copy_remote_tree_to_local(
        &sftp,
        src,
        dst,
        src,
        dst,
        spec.source_path.ends_with('/'),
    )
}

fn copy_local_symlink(from: &Path, to: &Path) -> Result<()> {
    if let Some(p) = to.parent() {
        mkdir_p_local(p, 0o755)?;
    }
    let target = fs::read_link(from).map_err(|e| GraphRunError::msg(format!("readlink: {e}")))?;
    let _ = fs::remove_file(to);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, to).map_err(|e| GraphRunError::msg(format!("symlink: {e}")))?;
        Ok(())
    }
    #[cfg(not(unix))]
    Err(GraphRunError::msg("local symlink copy only on Unix"))
}

fn copy_local_tree_to_local(src: &Path, dst: &Path, src_contents_only: bool) -> Result<()> {
    let meta = fs::symlink_metadata(src)
        .map_err(|e| GraphRunError::msg(format!("stat {}: {e}", src.display())))?;
    if meta.is_symlink() {
        return copy_local_symlink(src, dst);
    }
    if meta.is_file() {
        if let Some(p) = dst.parent() {
            mkdir_p_local(p, 0o755)?;
        }
        fs::copy(src, dst).map_err(|e| GraphRunError::msg(format!("copy: {e}")))?;
        #[cfg(unix)]
        {
            let perm = meta.permissions();
            let _ = fs::set_permissions(dst, perm);
        }
        if let Ok(m) = meta.modified() {
            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                let ft = filetime::FileTime::from_unix_time(d.as_secs() as i64, 0);
                let _ = filetime::set_file_mtime(dst, ft);
            }
        }
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(GraphRunError::msg("unsupported local file type"));
    }

    if src_contents_only {
        mkdir_p_local(dst, 0o755)?;
        for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
            let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
            copy_local_tree_to_local(&ent.path(), &dst.join(ent.file_name()), false)?;
        }
        return Ok(());
    }

    mkdir_p_local(dst, 0o755)?;
    for ent in fs::read_dir(src).map_err(|e| GraphRunError::msg(format!("readdir: {e}")))? {
        let ent = ent.map_err(|e| GraphRunError::msg(format!("readdir: {e}")))?;
        copy_local_tree_to_local(&ent.path(), &dst.join(ent.file_name()), false)?;
    }
    Ok(())
}

fn copy_local_to_local(
    _src_srv: &Server,
    _dst_srv: &Server,
    spec: &TransferSpec,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let src_s = expand_local_transfer_path(&spec.source_path, workspace_root)?;
    let dst_s = expand_local_transfer_path(&spec.dest_path, workspace_root)?;
    log_transfer_paths("local->local", spec, &src_s, &dst_s);
    let src = Path::new(&src_s);
    let dst = Path::new(&dst_s);
    copy_local_tree_to_local(src, dst, spec.source_path.ends_with('/'))
}

/// Copy files/directories between two servers (each `kind` may be `local` or `remote`).
/// Trailing slash on `source_path` means “copy directory contents into dest” (like `rsync` source slash).
pub fn run_transfer(
    bundle: &ConfigBundle,
    spec: &TransferSpec,
    timeout_secs: Option<u64>,
    workspace_root: Option<&Path>,
) -> Result<()> {
    let src_srv = bundle.servers.get(&spec.source_server_id).ok_or_else(|| {
        GraphRunError::msg(format!("transfer: unknown source_server_id {:?}", spec.source_server_id))
    })?;
    let dst_srv = bundle.servers.get(&spec.dest_server_id).ok_or_else(|| {
        GraphRunError::msg(format!("transfer: unknown dest_server_id {:?}", spec.dest_server_id))
    })?;

    let timeout_ms: u32 = timeout_secs
        .unwrap_or(300)
        .saturating_mul(1000)
        .min(u64::from(u32::MAX)) as u32;

    log::debug!(
        target: "graph_run",
        "transfer run_transfer: kinds=({}, {}) workspace_root_for_expand={:?} cwd={:?}",
        src_srv.kind,
        dst_srv.kind,
        workspace_root.map(|p| p.display().to_string()),
        std::env::current_dir().map(|p| p.display().to_string()),
    );

    match (src_srv.kind.as_str(), dst_srv.kind.as_str()) {
        ("local", "local") => copy_local_to_local(src_srv, dst_srv, spec, workspace_root),
        ("local", "remote") => copy_local_to_remote(src_srv, dst_srv, spec, timeout_ms, workspace_root),
        ("remote", "local") => copy_remote_to_local(src_srv, dst_srv, spec, timeout_ms, workspace_root),
        ("remote", "remote") => copy_remote_to_remote(src_srv, dst_srv, spec, timeout_ms, workspace_root),
        (a, b) => Err(GraphRunError::msg(format!(
            "transfer: unsupported server kind pair ({a}, {b}) (use local or remote)"
        ))),
    }
}

#[cfg(test)]
mod transfer_unit_tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::{ConfigBundle, Server, TransferSpec, WorkflowFile};

    fn empty_bundle(servers: HashMap<String, Server>) -> ConfigBundle {
        ConfigBundle {
            servers,
            shells: HashMap::new(),
            commands: HashMap::new(),
            tasks: HashMap::new(),
            workflow: WorkflowFile {
                nodes: vec![],
                edges: vec![],
            },
            explicit_control_nodes: std::collections::HashSet::new(),
            explicit_task_nodes: std::collections::HashSet::new(),
        }
    }

    fn local_server(id: &str) -> Server {
        Server {
            id: id.into(),
            kind: "local".into(),
            description: None,
            transport: None,
            host: None,
            port: None,
            user: None,
            timeout: None,
            password: None,
            password_env: None,
        }
    }

    /// When `readdir` returns absolute paths, `dst.join(name)` would replace the destination root
    /// (Rust `Path` rules); we map under `dst_root` instead.
    #[test]
    fn absolute_readdir_name_maps_under_local_dst() {
        let src_root = Path::new("/config/tmp");
        let dst_root = Path::new("/Users/me/project/.workspace");
        let parent = Path::new("/config/tmp");
        let name = Path::new("/config/tmp/hi.txt");
        let child_src = remote_sftp_entry_path(parent, name);
        let child_dst = path_under_mirror_root(src_root, dst_root, &child_src).unwrap();
        assert_eq!(child_dst, Path::new("/Users/me/project/.workspace/hi.txt"));
    }

    #[test]
    fn relative_readdir_name_still_works() {
        let src_root = Path::new("/config/tmp");
        let dst_root = Path::new("/out");
        let child_src = remote_sftp_entry_path(Path::new("/config/tmp"), Path::new("hi.txt"));
        let child_dst = path_under_mirror_root(src_root, dst_root, &child_src).unwrap();
        assert_eq!(child_dst, Path::new("/out/hi.txt"));
    }

    #[test]
    fn path_under_mirror_root_same_as_src_root_is_dst_root() {
        let src_root = Path::new("/data/tree");
        let dst_root = Path::new("/out/ws");
        let got = path_under_mirror_root(src_root, dst_root, src_root).unwrap();
        assert_eq!(got, dst_root);
    }

    #[test]
    fn path_under_mirror_root_not_under_src_errors() {
        let src_root = Path::new("/a/b");
        let dst_root = Path::new("/c");
        let err = path_under_mirror_root(src_root, dst_root, Path::new("/other/x")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not under source root"), "{msg}");
    }

    #[test]
    fn expand_graph_run_workspace_and_tmp() {
        let ws = Path::new("rel/ws");
        assert_eq!(
            expand_graph_run_tokens("$GRAPH_RUN_WORKSPACE/x", Some(ws)).unwrap(),
            "rel/ws/x"
        );
        assert_eq!(
            expand_graph_run_tokens("p/$GRAPH_RUN_TMP", Some(ws)).unwrap(),
            "p/rel/ws/tmp"
        );
    }

    #[test]
    fn expand_graph_run_workspace_and_tmp_in_one_path() {
        let ws = Path::new("nested_ws_root");
        let got = expand_graph_run_tokens(
            "$GRAPH_RUN_WORKSPACE/a/$GRAPH_RUN_TMP/b",
            Some(ws),
        )
        .unwrap();
        assert_eq!(got, "nested_ws_root/a/nested_ws_root/tmp/b");
    }

    #[test]
    fn expand_graph_run_workspace_without_workspace_errors() {
        let err = expand_graph_run_tokens("x$GRAPH_RUN_WORKSPACEx", None).unwrap_err();
        assert!(
            err.to_string().contains("GRAPH_RUN_WORKSPACE"),
            "{}",
            err
        );
    }

    #[test]
    fn expand_graph_run_tmp_without_workspace_errors() {
        let err = expand_graph_run_tokens("$GRAPH_RUN_TMP", None).unwrap_err();
        assert!(err.to_string().contains("GRAPH_RUN_TMP"), "{}", err);
    }

    #[test]
    fn expand_local_transfer_path_workspace_segment() {
        let ws = Path::new("my/ws");
        assert_eq!(
            expand_local_transfer_path("$GRAPH_RUN_WORKSPACE/only", Some(ws)).unwrap(),
            "my/ws/only"
        );
    }

    #[cfg(unix)]
    #[test]
    fn expand_local_transfer_path_combines_workspace_and_home() {
        let ws = Path::new("/ws");
        let home = std::env::var("HOME").expect("HOME for test");
        let got = expand_local_transfer_path("$GRAPH_RUN_WORKSPACE/$HOME/rel", Some(ws)).unwrap();
        assert!(got.contains("/ws/"), "{got}");
        assert!(got.contains(&home), "{got}");
    }

    #[test]
    fn file_stat_perm_none_is_not_dir_reg_or_link() {
        let st = FileStat {
            size: None,
            uid: None,
            gid: None,
            perm: None,
            atime: None,
            mtime: None,
        };
        assert!(!is_dir(&st));
        assert!(!is_reg(&st));
        assert!(!is_lnk(&st));
        assert_eq!(mode_for_file(&st), 0o644);
        assert_eq!(mode_for_mkdir(&st), 0o755);
    }

    #[test]
    fn file_stat_kind_helpers() {
        let dir_st = FileStat {
            size: None,
            uid: None,
            gid: None,
            perm: Some(0o040755),
            atime: None,
            mtime: None,
        };
        assert!(is_dir(&dir_st));
        assert!(!is_reg(&dir_st));
        assert!(!is_lnk(&dir_st));
        assert_eq!(mode_for_mkdir(&dir_st), 0o755);

        let reg_st = FileStat {
            size: Some(3),
            uid: None,
            gid: None,
            perm: Some(0o100644),
            atime: None,
            mtime: Some(1),
        };
        assert!(is_reg(&reg_st));
        assert!(!is_dir(&reg_st));
        assert_eq!(mode_for_file(&reg_st), 0o644);

        let lnk_st = FileStat {
            size: None,
            uid: None,
            gid: None,
            perm: Some(0o120777),
            atime: None,
            mtime: None,
        };
        assert!(is_lnk(&lnk_st));
    }

    #[test]
    fn ssh_connect_session_errors_when_remote_has_no_host() {
        let srv = Server {
            id: "r".into(),
            kind: "remote".into(),
            description: None,
            transport: Some("ssh".into()),
            host: None,
            port: None,
            user: Some("u".into()),
            timeout: None,
            password: None,
            password_env: None,
        };
        let e = match ssh_connect_session(&srv, 100) {
            Ok(_) => panic!("expected error when remote server has no host"),
            Err(e) => e,
        };
        let msg = e.to_string();
        assert!(msg.contains("no host"), "{msg}");
    }

    #[test]
    fn ssh_connect_session_rejects_non_remote() {
        let srv = Server {
            id: "loc".into(),
            kind: "local".into(),
            description: None,
            transport: None,
            host: Some("127.0.0.1".into()),
            port: Some(22),
            user: Some("u".into()),
            timeout: None,
            password: None,
            password_env: None,
        };
        let err = match ssh_connect_session(&srv, 1000) {
            Ok(_) => panic!("expected local server to be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("not remote"), "{}", err);
    }

    #[test]
    fn expand_local_transfer_path_plain_path_unchanged() {
        assert_eq!(
            expand_local_transfer_path("/tmp/plain.txt", None).unwrap(),
            "/tmp/plain.txt"
        );
    }

    #[test]
    fn expand_local_transfer_path_errors_when_home_unset() {
        use std::sync::Mutex;
        static HOME_LOCK: Mutex<()> = Mutex::new(());
        let _g = HOME_LOCK.lock().expect("home env lock");
        let old = std::env::var("HOME").ok();
        std::env::remove_var("HOME");
        let err = expand_local_transfer_path("$HOME/x", None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HOME"), "{msg}");
        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn run_transfer_unknown_servers_and_unsupported_kinds() {
        let mut servers = HashMap::new();
        servers.insert("a".into(), local_server("a"));
        servers.insert("b".into(), local_server("b"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "missing".into(),
            dest_server_id: "a".into(),
            source_path: "/x".into(),
            dest_path: "/y".into(),
        };
        let e = run_transfer(&bundle, &spec, None, None).unwrap_err();
        assert!(e.to_string().contains("unknown source_server_id"), "{e}");

        let mut servers = HashMap::new();
        servers.insert("a".into(), local_server("a"));
        servers.insert("b".into(), local_server("b"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "a".into(),
            dest_server_id: "ghost".into(),
            source_path: "/x".into(),
            dest_path: "/y".into(),
        };
        let e = run_transfer(&bundle, &spec, None, None).unwrap_err();
        assert!(e.to_string().contains("unknown dest_server_id"), "{e}");

        let mut servers = HashMap::new();
        let mut s1 = local_server("a");
        s1.kind = "sftp".into();
        servers.insert("a".into(), s1);
        servers.insert("b".into(), local_server("b"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "a".into(),
            dest_server_id: "b".into(),
            source_path: "/x".into(),
            dest_path: "/y".into(),
        };
        let e = run_transfer(&bundle, &spec, Some(1), None).unwrap_err();
        assert!(e.to_string().contains("unsupported server kind pair"), "{e}");
    }

    #[test]
    fn run_transfer_timeout_secs_saturates_to_u32_max_ms() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("one.txt");
        fs::write(&src, b"x").unwrap();
        let dst = tmp.path().join("two.txt");
        let mut servers = HashMap::new();
        servers.insert("a".into(), local_server("a"));
        servers.insert("b".into(), local_server("b"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "a".into(),
            dest_server_id: "b".into(),
            source_path: src.to_string_lossy().into_owned(),
            dest_path: dst.to_string_lossy().into_owned(),
        };
        run_transfer(&bundle, &spec, Some(u64::MAX), None).unwrap();
        assert_eq!(fs::read_to_string(dst).unwrap(), "x");
    }

    #[test]
    fn run_transfer_local_to_local_file_and_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src_dir = tmp.path().join("from");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("hello.txt"), b"abc").unwrap();
        let nested = src_dir.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("inner.toml"), b"v").unwrap();

        let dst_base = tmp.path().join("dest");
        fs::create_dir_all(&dst_base).unwrap();

        let mut servers = HashMap::new();
        servers.insert("src".into(), local_server("src"));
        servers.insert("dst".into(), local_server("dst"));
        let bundle = empty_bundle(servers);

        let spec_file = TransferSpec {
            source_server_id: "src".into(),
            dest_server_id: "dst".into(),
            source_path: src_dir.join("hello.txt").to_string_lossy().into_owned(),
            dest_path: dst_base.join("copied.txt").to_string_lossy().into_owned(),
        };
        run_transfer(&bundle, &spec_file, Some(60), None).unwrap();
        assert_eq!(fs::read_to_string(dst_base.join("copied.txt")).unwrap(), "abc");

        let dst_tree = dst_base.join("tree");
        let spec_dir = TransferSpec {
            source_server_id: "src".into(),
            dest_server_id: "dst".into(),
            source_path: src_dir.to_string_lossy().into_owned(),
            dest_path: dst_tree.to_string_lossy().into_owned(),
        };
        run_transfer(&bundle, &spec_dir, Some(60), None).unwrap();
        assert_eq!(
            fs::read_to_string(dst_tree.join("hello.txt")).unwrap(),
            "abc"
        );
        assert_eq!(
            fs::read_to_string(dst_tree.join("sub").join("inner.toml")).unwrap(),
            "v"
        );

        let dst_flat = dst_base.join("flat");
        let spec_contents = TransferSpec {
            source_server_id: "src".into(),
            dest_server_id: "dst".into(),
            source_path: format!("{}/", src_dir.to_string_lossy()),
            dest_path: dst_flat.to_string_lossy().into_owned(),
        };
        run_transfer(&bundle, &spec_contents, Some(60), None).unwrap();
        assert_eq!(
            fs::read_to_string(dst_flat.join("hello.txt")).unwrap(),
            "abc"
        );
    }

    #[test]
    fn run_transfer_local_to_local_with_workspace_tokens() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = tmp.path().join("ws");
        fs::create_dir_all(ws.join("tmp")).unwrap();
        fs::write(ws.join("marker"), b"ok").unwrap();

        let mut servers = HashMap::new();
        servers.insert("src".into(), local_server("src"));
        servers.insert("dst".into(), local_server("dst"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "src".into(),
            dest_server_id: "dst".into(),
            source_path: "$GRAPH_RUN_WORKSPACE/marker".into(),
            dest_path: "$GRAPH_RUN_TMP/out.txt".into(),
        };
        run_transfer(&bundle, &spec, None, Some(&ws)).unwrap();
        assert_eq!(fs::read_to_string(ws.join("tmp").join("out.txt")).unwrap(), "ok");
    }

    #[cfg(unix)]
    #[test]
    fn run_transfer_local_to_local_symlink() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("real.txt");
        fs::write(&target, b"link-body").unwrap();
        let link = tmp.path().join("via_link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let out = tmp.path().join("out_link.txt");
        let mut servers = HashMap::new();
        servers.insert("src".into(), local_server("src"));
        servers.insert("dst".into(), local_server("dst"));
        let bundle = empty_bundle(servers);
        let spec = TransferSpec {
            source_server_id: "src".into(),
            dest_server_id: "dst".into(),
            source_path: link.to_string_lossy().into_owned(),
            dest_path: out.to_string_lossy().into_owned(),
        };
        run_transfer(&bundle, &spec, Some(30), None).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "link-body");
        assert!(fs::symlink_metadata(&out).unwrap().is_symlink());
    }
}

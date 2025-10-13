use core::fmt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use std::{io, marker, mem, ptr};

use futures_util::{Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::windows::named_pipe;
use windows_sys::Win32::Foundation::{
    ERROR_PIPE_BUSY, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HLOCAL, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ACCESS_MODE, EXPLICIT_ACCESS_W, SET_ACCESS, SetEntriesInAclW, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_TYPE,
};
use windows_sys::Win32::Security::{
    ACL, AllocateAndInitializeSid, FreeSid, InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR,
    PSID, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SID_IDENTIFIER_AUTHORITY,
    SetSecurityDescriptorDacl,
};
use windows_sys::Win32::System::Memory::{LPTR, LocalAlloc};
use windows_sys::Win32::System::SystemServices::{
    SECURITY_DESCRIPTOR_REVISION, SECURITY_WORLD_RID,
};

use crate::{IntoIpcPath, OnConflict, ServerId};

#[derive(Debug)]
enum NamedPipe {
    Server(named_pipe::NamedPipeServer),
    Client(named_pipe::NamedPipeClient),
}

const PIPE_AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(5);

impl<T> ServerId<T>
where
    T: Into<String> + Send,
{
    pub(crate) fn into_ipc_path(self) -> io::Result<PathBuf> {
        Ok(PathBuf::from(format!(
            r"\\.\pipe\{}",
            self.id.into().replace('/', "\\")
        )))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    path: PathBuf,
    security_attributes: SecurityAttributes,
    created_listener: bool,
}

impl Endpoint {
    fn create_listener(&mut self) -> io::Result<named_pipe::NamedPipeServer> {
        let server = unsafe {
            named_pipe::ServerOptions::new()
                .first_pipe_instance(!self.created_listener)
                .reject_remote_clients(true)
                .access_inbound(true)
                .access_outbound(true)
                .in_buffer_size(65536)
                .out_buffer_size(65536)
                .create_with_security_attributes_raw(
                    &self.path,
                    self.security_attributes.as_ptr().cast_mut().cast(),
                )
        }?;
        self.created_listener = true;

        Ok(server)
    }

    pub(crate) async fn connect(path: impl IntoIpcPath) -> io::Result<Connection> {
        let path = path.into_ipc_path()?;

        // There is not async equivalent of waiting for a named pipe in Windows,
        // so we keep trying or sleeping for a bit, until we hit a timeout
        let attempt_start = Instant::now();
        let client = loop {
            match named_pipe::ClientOptions::new()
                .read(true)
                .write(true)
                .open(&path)
            {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    if attempt_start.elapsed() < PIPE_AVAILABILITY_TIMEOUT {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        continue;
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        };

        Ok(Connection::wrap(NamedPipe::Client(client)))
    }

    pub(crate) fn incoming(self) -> io::Result<IpcStream> {
        IpcStream::new(self)
    }

    pub(crate) fn security_attributes(mut self, security_attributes: SecurityAttributes) -> Self {
        self.security_attributes = security_attributes;
        self
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn new(path: impl IntoIpcPath, _on_conflict: OnConflict) -> io::Result<Self> {
        Ok(Self {
            path: path.into_ipc_path()?,
            security_attributes: SecurityAttributes::empty(),
            created_listener: false,
        })
    }
}

pub(crate) struct IpcStream {
    inner: Pin<Box<dyn Stream<Item = io::Result<Connection>> + Send>>,
}

impl fmt::Debug for IpcStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<stream>")
    }
}

impl IpcStream {
    pub(crate) fn new(mut endpoint: Endpoint) -> io::Result<Self> {
        let pipe = endpoint.create_listener()?;

        let stream = futures_util::stream::try_unfold(
            (pipe, endpoint),
            |(listener, mut endpoint)| async move {
                listener.connect().await?;
                let new_listener = endpoint.create_listener()?;
                let conn = Connection::wrap(NamedPipe::Server(listener));

                Ok(Some((conn, (new_listener, endpoint))))
            },
        );
        Ok(Self {
            inner: Box::pin(stream),
        })
    }
}

impl Stream for IpcStream {
    type Item = io::Result<Connection>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = Pin::into_inner(self);
        Pin::new(&mut this.inner).poll_next_unpin(cx)
    }
}

#[derive(Debug)]
pub(crate) struct Connection {
    inner: NamedPipe,
}

impl Connection {
    /// Wraps an existing named pipe
    fn wrap(pipe: NamedPipe) -> Self {
        Self { inner: pipe }
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = Pin::into_inner(self);
        match this.inner {
            NamedPipe::Client(ref mut c) => Pin::new(c).poll_read(ctx, buf),
            NamedPipe::Server(ref mut s) => Pin::new(s).poll_read(ctx, buf),
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let this = Pin::into_inner(self);
        match this.inner {
            NamedPipe::Client(ref mut c) => Pin::new(c).poll_write(ctx, buf),
            NamedPipe::Server(ref mut s) => Pin::new(s).poll_write(ctx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        match this.inner {
            NamedPipe::Client(ref mut c) => Pin::new(c).poll_flush(ctx),
            NamedPipe::Server(ref mut s) => Pin::new(s).poll_flush(ctx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let this = Pin::into_inner(self);
        match this.inner {
            NamedPipe::Client(ref mut c) => Pin::new(c).poll_shutdown(ctx),
            NamedPipe::Server(ref mut s) => Pin::new(s).poll_shutdown(ctx),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SecurityAttributes {
    attributes: Option<InnerAttributes>,
}

const DEFAULT_SECURITY_ATTRIBUTES: SecurityAttributes = SecurityAttributes {
    attributes: Some(InnerAttributes {
        descriptor: SecurityDescriptor {
            descriptor_ptr: ptr::null_mut(),
        },
        acl: Acl {
            acl_ptr: ptr::null_mut(),
        },
        attrs: SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 0,
        },
    }),
};

impl SecurityAttributes {
    pub(crate) unsafe fn as_ptr(&mut self) -> *const SECURITY_ATTRIBUTES {
        unsafe {
            match self.attributes.as_mut() {
                Some(attributes) => attributes.as_ptr(),
                None => ptr::null_mut(),
            }
        }
    }

    pub(crate) fn empty() -> Self {
        DEFAULT_SECURITY_ATTRIBUTES
    }

    pub(crate) fn allow_everyone_connect() -> io::Result<Self> {
        Self::allow_everyone_create()
    }

    pub(crate) fn mode(self, _mode: u16) -> io::Result<Self> {
        // for now, does nothing.
        Ok(self)
    }

    pub(crate) fn allow_everyone_create() -> io::Result<Self> {
        let attributes = Some(InnerAttributes::allow_everyone(
            GENERIC_READ | GENERIC_WRITE,
        )?);
        Ok(Self { attributes })
    }
}

unsafe impl Send for SecurityAttributes {}

struct Sid {
    sid_ptr: PSID,
}

impl Sid {
    fn everyone_sid() -> io::Result<Self> {
        let mut sid_ptr = ptr::null_mut();
        let world_sid_authority = SID_IDENTIFIER_AUTHORITY {
            Value: [0, 0, 0, 0, 0, 1],
        };
        let result = unsafe {
            AllocateAndInitializeSid(
                &world_sid_authority,
                1,
                SECURITY_WORLD_RID as _,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                &mut sid_ptr,
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self { sid_ptr })
        }
    }

    // Unsafe - the returned pointer is only valid for the lifetime of self.
    unsafe fn as_ptr(&self) -> PSID {
        self.sid_ptr
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        if !self.sid_ptr.is_null() {
            unsafe {
                FreeSid(self.sid_ptr);
            }
        }
    }
}

struct AceWithSid<'a> {
    explicit_access: EXPLICIT_ACCESS_W,
    _marker: marker::PhantomData<&'a Sid>,
}

impl<'a> AceWithSid<'a> {
    fn new(sid: &'a Sid, trustee_type: TRUSTEE_TYPE) -> Self {
        let mut explicit_access = unsafe { mem::zeroed::<EXPLICIT_ACCESS_W>() };
        explicit_access.Trustee.TrusteeForm = TRUSTEE_IS_SID;
        explicit_access.Trustee.TrusteeType = trustee_type;
        explicit_access.Trustee.ptstrName = unsafe { sid.as_ptr().cast() };

        AceWithSid {
            explicit_access,
            _marker: marker::PhantomData,
        }
    }

    fn set_access_mode(&mut self, access_mode: ACCESS_MODE) -> &mut Self {
        self.explicit_access.grfAccessMode = access_mode;
        self
    }

    fn set_access_permissions(&mut self, access_permissions: u32) -> &mut Self {
        self.explicit_access.grfAccessPermissions = access_permissions;
        self
    }

    fn allow_inheritance(&mut self, inheritance_flags: u32) -> &mut Self {
        self.explicit_access.grfInheritance = inheritance_flags;
        self
    }
}

#[derive(Debug, Clone)]
struct Acl {
    acl_ptr: *const ACL,
}

impl Acl {
    fn empty() -> io::Result<Self> {
        Self::new(&mut [])
    }

    fn new(entries: &mut [AceWithSid<'_>]) -> io::Result<Self> {
        let mut acl_ptr = ptr::null_mut();
        let result = unsafe {
            SetEntriesInAclW(
                entries.len() as u32,
                entries.as_mut_ptr().cast(),
                ptr::null_mut(),
                &mut acl_ptr,
            )
        };

        if result != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(result as i32));
        }

        Ok(Self { acl_ptr })
    }

    unsafe fn as_ptr(&self) -> *const ACL {
        self.acl_ptr
    }
}

impl Drop for Acl {
    fn drop(&mut self) {
        if !self.acl_ptr.is_null() {
            unsafe { LocalFree(self.acl_ptr as HLOCAL) };
        }
    }
}

#[derive(Debug, Clone)]
struct SecurityDescriptor {
    descriptor_ptr: PSECURITY_DESCRIPTOR,
}

impl SecurityDescriptor {
    fn new() -> io::Result<Self> {
        let descriptor_ptr = unsafe { LocalAlloc(LPTR, mem::size_of::<SECURITY_DESCRIPTOR>()) }
            as PSECURITY_DESCRIPTOR;
        if descriptor_ptr.is_null() {
            return Err(io::Error::other("Failed to allocate security descriptor"));
        }

        if unsafe {
            InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION) == 0
        } {
            return Err(io::Error::last_os_error());
        };

        Ok(Self { descriptor_ptr })
    }

    fn set_dacl(&mut self, acl: &Acl) -> io::Result<()> {
        if unsafe {
            SetSecurityDescriptorDacl(self.descriptor_ptr, true as i32, acl.as_ptr(), false as i32)
                == 0
        } {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    unsafe fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.descriptor_ptr
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.descriptor_ptr.is_null() {
            unsafe { LocalFree(self.descriptor_ptr as HLOCAL) };
            self.descriptor_ptr = ptr::null_mut();
        }
    }
}

#[derive(Clone)]
struct InnerAttributes {
    descriptor: SecurityDescriptor,
    acl: Acl,
    attrs: SECURITY_ATTRIBUTES,
}

impl fmt::Debug for InnerAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InnerAttributes")
            .field("descriptor", &self.descriptor)
            .field("acl", &self.acl)
            .field("attrs", &"<attrs>")
            .finish()
    }
}

impl InnerAttributes {
    fn empty() -> io::Result<Self> {
        let descriptor = SecurityDescriptor::new()?;
        let mut attrs = unsafe { mem::zeroed::<SECURITY_ATTRIBUTES>() };
        attrs.nLength = mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        attrs.lpSecurityDescriptor = unsafe { descriptor.as_ptr() };
        attrs.bInheritHandle = false as i32;

        let acl = Acl::empty().expect("this should never fail");

        Ok(Self {
            acl,
            descriptor,
            attrs,
        })
    }

    fn allow_everyone(permissions: u32) -> io::Result<Self> {
        let mut attributes = Self::empty()?;
        let sid = Sid::everyone_sid()?;

        let mut everyone_ace = AceWithSid::new(&sid, TRUSTEE_IS_WELL_KNOWN_GROUP);
        everyone_ace
            .set_access_mode(SET_ACCESS)
            .set_access_permissions(permissions)
            .allow_inheritance(false as u32);

        let mut entries = vec![everyone_ace];
        attributes.acl = Acl::new(&mut entries)?;
        attributes.descriptor.set_dacl(&attributes.acl)?;

        Ok(attributes)
    }

    unsafe fn as_ptr(&mut self) -> *const SECURITY_ATTRIBUTES {
        &mut self.attrs
    }
}

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
};

use socket2::{Domain, Protocol, Socket, Type};

pub(crate) fn bind_udp(addr: SocketAddr) -> io::Result<UdpSocket> {
    match bind(addr) {
        Err(_) if addr.is_ipv6() && addr.ip().is_unspecified() => {
            bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, addr.port())))
        },
        result => result,
    }
}

fn bind(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }

    socket.bind(&addr.into())?;

    Ok(socket.into())
}

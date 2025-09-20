FROM ros:noetic-ros-base
ADD https://github.com/just-containers/s6-overlay/releases/download/v3.2.1.0/s6-overlay-noarch.tar.xz /tmp
RUN tar -C / -Jxpf /tmp/s6-overlay-noarch.tar.xz
ADD https://github.com/just-containers/s6-overlay/releases/download/v3.2.1.0/s6-overlay-x86_64.tar.xz /tmp
RUN tar -C / -Jxpf /tmp/s6-overlay-x86_64.tar.xz

RUN apt-get update && \
    apt-get install -y \
    ros-$(rosversion -d)-turtlesim \
    websockify \
    inetutils-tools \
    boxes \
    pwgen \
    xvfb \
    x11vnc && \
    mkdir -p /usr/share/novnc/utils/websockify && \
    curl -sSL https://github.com/novnc/noVNC/archive/master.tar.gz | tar xfz - --strip 1 -C /usr/share/novnc && \
    mv /usr/share/novnc/utils/novnc_proxy /usr/bin/novnc_server && \
    mkdir /var/www

COPY vnc.html /var/www/vnc.html

COPY s6-rc.d /etc/s6-overlay/s6-rc.d
ENTRYPOINT ["/init"]
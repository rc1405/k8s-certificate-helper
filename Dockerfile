FROM ubuntu:latest
COPY target/release/k8s-certificate /bin/k8s-certificate
RUN chmod a+x /bin/k8s-certificate
ENTRYPOINT [ "k8s-certificate" ]
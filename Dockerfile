FROM ubuntu:latest
COPY target/release/certificate-helper /bin/certificate-helper
RUN chmod a+x /bin/certificate-helper
ENTRYPOINT [ "certificate-helper" ]
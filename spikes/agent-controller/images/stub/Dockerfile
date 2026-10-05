FROM python:3.12-slim
COPY stub.py /stub.py
# The run's workspace volume is mounted here. Docker copies this directory's
# ownership into a new volume, so the unprivileged stub can write to it.
RUN mkdir /workspace && chown nobody:nogroup /workspace
USER nobody
WORKDIR /workspace
ENTRYPOINT ["python3", "-u", "/stub.py"]

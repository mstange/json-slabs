for (const sig of ['SIGTERM', 'SIGINT', 'SIGHUP']) {
  // Do an ordinary exit if vitest tries to kill this process,
  // so that buffered jitdump files (for profiling) are flushed.
  process.on(sig, () => process.exit(0));
}

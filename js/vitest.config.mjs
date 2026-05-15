export default {
  // Make it possible to profile the benchmark using samply
  // and jitdump.
  //
  // ```
  // JSLB_BENCH_FIXTURE=$HOME/Downloads/as-jslb-new.jslb \
  // samply record \
  // node ./node_modules/vitest/vitest.mjs bench --run \
  //      --no-file-parallelism \
  //      --execArgv=--perf-prof \
  //      --execArgv=--interpreted-frames-native-stack \
  //      --execArgv=--perf-prof-path=/tmp
  // ```
  test: {
    pool: 'forks',
    fileParallelism: false,
    setupFiles: ['./jit-prof-setup.mjs'],
    teardownTimeout: 60_000,
  },
};

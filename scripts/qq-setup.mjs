import { startQrConnect } from '@tencent-connect/qqbot-connector';

const emit = (event) => {
  process.stdout.write(`${JSON.stringify(event)}\n`);
};

let stopped = false;
const stop = startQrConnect(
  {
    onQrDisplayed(url) {
      emit({ type: 'qr_url', url });
    },
    onQrExpired() {
      emit({ type: 'expired' });
    },
    onSuccess(credentials) {
      if (!stopped) {
        stopped = true;
        // Credentials only cross the private stdout pipe to the Rust parent.
        // Never log this object: stdout is consumed by the Rust secret writer.
        emit({
          type: 'credentials',
          credentials: credentials.map(({ appId, appSecret, userOpenid }) => ({
            appId,
            appSecret,
            ...(userOpenid ? { userOpenid } : {}),
          })),
        });
      }
    },
    onFailure(error) {
      if (!stopped) {
        stopped = true;
        emit({ type: 'error', message: error instanceof Error ? error.message : String(error) });
        process.exitCode = 1;
      }
    },
  },
  { displayQrCodeToConsole: false },
);

const shutdown = () => {
  if (!stopped) {
    stopped = true;
    stop();
  }
};
process.once('SIGINT', shutdown);
process.once('SIGTERM', shutdown);

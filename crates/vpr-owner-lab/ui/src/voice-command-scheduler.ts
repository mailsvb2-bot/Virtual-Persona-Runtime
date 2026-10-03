type PlaybackBarrier = {
  promise: Promise<void>;
  resolve: () => void;
};

export class PlaybackAwareCommandScheduler<T> {
  private currentPlayback: PlaybackBarrier | null = null;
  private generation = 0;
  private pendingCommands = 0;
  private queueTail: Promise<void> = Promise.resolve();

  constructor(private readonly send: (command: T) => Promise<void>) {}

  get hasActivePlayback(): boolean {
    return this.currentPlayback !== null;
  }

  get hasPendingPlayback(): boolean {
    return this.pendingCommands > 0;
  }

  dispatch(command: T): Promise<boolean> {
    this.pendingCommands += 1;
    const generation = this.generation;
    let resolveResult!: (sent: boolean) => void;
    let rejectResult!: (error: unknown) => void;
    const result = new Promise<boolean>((resolve, reject) => {
      resolveResult = resolve;
      rejectResult = reject;
    });

    const run = async (): Promise<void> => {
      if (generation !== this.generation) {
        resolveResult(false);
        return;
      }

      let release!: () => void;
      const barrier: PlaybackBarrier = {
        promise: new Promise<void>((done) => {
          release = done;
        }),
        resolve: () => release(),
      };
      this.currentPlayback = barrier;

      try {
        await this.send(command);
        if (generation !== this.generation) {
          if (this.currentPlayback === barrier) {
            this.currentPlayback = null;
            barrier.resolve();
          }
          resolveResult(false);
          return;
        }
        resolveResult(true);
        await barrier.promise;
      } catch (error) {
        if (this.currentPlayback === barrier) {
          this.currentPlayback = null;
          barrier.resolve();
        }
        if (generation === this.generation && this.pendingCommands > 0) {
          this.pendingCommands -= 1;
        }
        rejectResult(error);
      }
    };

    const scheduled = this.queueTail.then(run, run);
    this.queueTail = scheduled.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  playbackDone(): void {
    const barrier = this.currentPlayback;
    if (!barrier) return;
    this.currentPlayback = null;
    if (this.pendingCommands > 0) this.pendingCommands -= 1;
    barrier.resolve();
  }

  interrupt(): void {
    this.generation += 1;
    this.pendingCommands = 0;
    const barrier = this.currentPlayback;
    this.currentPlayback = null;
    barrier?.resolve();
  }
}

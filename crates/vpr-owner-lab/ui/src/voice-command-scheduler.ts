type PlaybackBarrier = {
  promise: Promise<void>;
  resolve: () => void;
};

export class PlaybackAwareCommandScheduler<T> {
  private currentPlayback: PlaybackBarrier | null = null;
  private generation = 0;

  constructor(private readonly send: (command: T) => Promise<void>) {}

  get hasActivePlayback(): boolean {
    return this.currentPlayback !== null;
  }

  async dispatch(command: T): Promise<boolean> {
    const generation = this.generation;
    const previous = this.currentPlayback;
    if (previous) {
      await previous.promise;
      if (generation !== this.generation) return false;
    }

    if (generation !== this.generation) return false;
    let resolve!: () => void;
    const barrier: PlaybackBarrier = {
      promise: new Promise<void>((done) => {
        resolve = done;
      }),
      resolve: () => resolve(),
    };
    this.currentPlayback = barrier;

    try {
      await this.send(command);
    } catch (error) {
      if (this.currentPlayback === barrier) {
        this.currentPlayback = null;
        barrier.resolve();
      }
      throw error;
    }

    if (generation !== this.generation) {
      if (this.currentPlayback === barrier) {
        this.currentPlayback = null;
        barrier.resolve();
      }
      return false;
    }
    return true;
  }

  playbackDone(): void {
    const barrier = this.currentPlayback;
    if (!barrier) return;
    this.currentPlayback = null;
    barrier.resolve();
  }

  interrupt(): void {
    this.generation += 1;
    const barrier = this.currentPlayback;
    this.currentPlayback = null;
    barrier?.resolve();
  }
}

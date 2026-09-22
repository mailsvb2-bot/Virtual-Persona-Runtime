export class PlaybackAwareCommandScheduler {
    send;
    currentPlayback = null;
    generation = 0;
    constructor(send) {
        this.send = send;
    }
    get hasActivePlayback() {
        return this.currentPlayback !== null;
    }
    async dispatch(command) {
        const generation = this.generation;
        const previous = this.currentPlayback;
        if (previous) {
            await previous.promise;
            if (generation !== this.generation)
                return false;
        }
        if (generation !== this.generation)
            return false;
        let resolve;
        const barrier = {
            promise: new Promise((done) => {
                resolve = done;
            }),
            resolve: () => resolve(),
        };
        this.currentPlayback = barrier;
        try {
            await this.send(command);
        }
        catch (error) {
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
    playbackDone() {
        const barrier = this.currentPlayback;
        if (!barrier)
            return;
        this.currentPlayback = null;
        barrier.resolve();
    }
    interrupt() {
        this.generation += 1;
        const barrier = this.currentPlayback;
        this.currentPlayback = null;
        barrier?.resolve();
    }
}

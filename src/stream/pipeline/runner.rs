use gst::prelude::*;

use anyhow::{anyhow, Context, Result};

use tokio::sync::broadcast;
use tracing::*;

#[derive(Debug)]
#[allow(dead_code)]
pub struct PipelineRunner {
    killswitch_sender: broadcast::Sender<String>,
    _killswitch_receiver: broadcast::Receiver<String>,
    _watcher_thread_handle: std::thread::JoinHandle<()>,
}

#[allow(dead_code)]
impl PipelineRunner {
    #[instrument(level = "debug")]
    pub fn new(pipeline: &gst::Pipeline, pipeline_id: uuid::Uuid) -> Self {
        let pipeline_weak = pipeline.downgrade();
        let (killswitch_sender, _killswitch_receiver) = broadcast::channel(1);
        Self {
            killswitch_sender: killswitch_sender.clone(),
            _killswitch_receiver,
            _watcher_thread_handle: std::thread::Builder::new()
                .name(format!("PipelineRunner-{pipeline_id}"))
                .spawn(move || {
                    if let Err(error) = PipelineRunner::runner(pipeline_weak, pipeline_id) {
                        // Any error here should interrupt the respective session!
                        error!("PipelineWatcher ended with error: {error}");
                        if let Err(reason) = killswitch_sender.send(error.to_string()) {
                            error!(
                                "Failed to broadcast error from PipelineWatcher. Reason: {reason}"
                            );
                        } else {
                            info!("Error sent to killswitch channel!");
                        }
                    } else {
                        info!("PipelineWatcher ended with no error.");
                    }
                })
                .unwrap(),
        }
    }

    #[instrument(level = "debug")]
    pub fn get_receiver(&self) -> broadcast::Receiver<String> {
        self.killswitch_sender.subscribe()
    }

    #[instrument(level = "debug")]
    pub fn is_running(&self) -> bool {
        !self._watcher_thread_handle.is_finished()
    }

    #[instrument(level = "debug")]
    fn runner(
        pipeline_weak: gst::glib::WeakRef<gst::Pipeline>,
        pipeline_id: uuid::Uuid,
    ) -> Result<()> {
        let pipeline = pipeline_weak
            .upgrade()
            .context("Unable to access the Pipeline from its weak reference")?;

        let bus = pipeline
            .bus()
            .context("Unable to access the pipeline bus")?;

        // Check if we need to break external loop.
        // Some cameras have a duplicated timestamp when starting.
        // to avoid restarting the camera once and once again,
        // this checks for a maximum of 10 lost before restarting.
        let mut previous_position: Option<gst::ClockTime> = None;
        let mut lost_timestamps: usize = 0;
        let max_lost_timestamps: usize = 15;

        'outer: loop {
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Restart pipeline if pipeline position do not change,
            // occur if usb connection is lost and gst do not detect it
            if let Some(position) = pipeline.query_position::<gst::ClockTime>() {
                previous_position = match previous_position {
                    Some(current_previous_position) => {
                        if current_previous_position.nseconds() != 0
                            && current_previous_position.nseconds() == position.nseconds()
                        {
                            lost_timestamps += 1;
                            warn!("Position did not change {lost_timestamps}");
                        } else {
                            // We are back in track, erase lost timestamps
                            lost_timestamps = 0;
                        }

                        if lost_timestamps > max_lost_timestamps {
                            error!("Pipeline lost too many timestamps (max. was {max_lost_timestamps}).");
                            let _ = pipeline.set_state(gst::State::Null);
                            while pipeline.current_state() != gst::State::Null {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                            let _ = pipeline.set_state(gst::State::Playing);
                            lost_timestamps = 0;
                        }

                        Some(position)
                    }
                    None => Some(position),
                }
            }

            /* Iterate messages on the bus until an error or EOS occurs,
             * although in this example the only error we'll hopefully
             * get is if the user closes the output window */
            while let Some(msg) = bus.timed_pop(gst::ClockTime::from_mseconds(100)) {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Eos(eos) => {
                        warn!("Received EndOfStream: {eos:#?}");
                        break 'outer;
                    }
                    MessageView::Error(error) => {
                        error!(
                            "Error from {:?}: {} ({:?})",
                            error.src().map(|s| s.path_string()),
                            error.error(),
                            error.debug()
                        );
                        pipeline.debug_to_dot_file_with_ts(
                            gst::DebugGraphDetails::all(),
                            format!("pipeline-error-{pipeline_id}"),
                        );
                        return Err(anyhow!(error.error()));
                    }
                    MessageView::StateChanged(state) => {
                        trace!(
                            "State changed from {:?}: {:?} to {:?} ({:?})",
                            state.src().map(|s| s.path_string()),
                            state.old(),
                            state.current(),
                            state.pending()
                        );
                    }
                    other_message => trace!("{other_message:#?}"),
                };
            }
        }

        Ok(())
    }
}

impl Drop for PipelineRunner {
    #[instrument(level = "debug")]
    fn drop(&mut self) {
        if let Err(reason) = self.killswitch_sender.send("Dropped.".to_string()) {
            error!(
                "Failed to send killswitch message while Dropping PipelineRunner. Reason: {reason:#?}"
            );
        }
    }
}

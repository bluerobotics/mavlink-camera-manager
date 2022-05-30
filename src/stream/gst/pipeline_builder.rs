use crate::{
    stream::types::VideoCaptureConfiguration,
    video::{
        types::{VideoEncodeType, VideoSourceType},
        video_source_gst::VideoSourceGstType,
    },
};
use crate::{
    stream::webrtc::{
        signalling_server::DEFAULT_SIGNALLING_ENDPOINT,
        turn_server::{DEFAULT_STUN_ENDPOINT, DEFAULT_TURN_ENDPOINT},
        utils::webrtc_usage_hint,
    },
    video_stream::types::VideoAndStreamInformation,
};
use log::*;
use simple_error::SimpleError;

#[derive(Clone, Debug, Default)]
pub struct Pipeline {
    pub description: String,
}

impl Pipeline {
    pub fn new(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<Self, SimpleError> {
        let encode = configuration.encode.clone();
        let endpoints = &video_and_stream_information.stream_information.endpoints;
        let video_source = &video_and_stream_information.video_source;

        let capability = Pipeline::build_capability_string(video_source, &encode, &configuration)?;

        let source = Pipeline::build_pipeline_source(video_source, &capability)?;
        let transcode = Pipeline::build_pipeline_transcode(video_source, &encode, is_webrtcsink)?;
        let payload = Pipeline::build_pipeline_payload(&encode, is_webrtcsink)?;
        let sink = Pipeline::build_pipeline_sink(&endpoints, is_webrtcsink)?;

        let description = format!("{source}{transcode}{payload}{sink}");

        info!("New pipeline built: {description:#?}");

        Ok(Pipeline { description })
    }

    fn build_capability_string(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<String, SimpleError> {
        let format = match video_source {
            VideoSourceType::Gst(_) => "video/x-raw",
            _ => match encode {
                VideoEncodeType::H264 => "video/x-h264",
                video_encode_type => {
                    return Err(SimpleError::new(format!(
                        "Unsupported VideoEncodeType: {video_encode_type:#?}",
                    )))
                }
            },
        };
        let pipeline_capability = format!(
            concat!(
                "{format},width={width},height={height},framerate={interval_denominator}/{interval_numerator}",
            ),
            format = format,
            width = configuration.width,
            height = configuration.height,
            interval_denominator = configuration.frame_interval.denominator,
            interval_numerator = configuration.frame_interval.numerator,
        );
        Ok(pipeline_capability)
    }

    fn build_pipeline_source(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<String, SimpleError> {
        let pipeline_source = match &video_and_stream_information.video_source {
            VideoSourceType::Gst(gst_source) => match &gst_source.source {
                VideoSourceGstType::Fake(pattern) => format!("videotestsrc pattern={pattern}"),
                VideoSourceGstType::Local(_) => {
                    return Err(SimpleError::new(format!(
                        "Unsupported GST source endpoint: {gst_source:#?}",
                    )));
                }
            },
            VideoSourceType::Local(local_device) => {
                format!("v4l2src device={}", &local_device.device_path)
            }
            video_source_type => {
                return Err(SimpleError::new(format!(
                    "Unsupported VideoSourceType: {video_source_type:#?}.",
                )));
            }
        };

        let capability = Pipeline::build_capability_string(&video_and_stream_information)?;
        Ok(format!("{pipeline_source} ! {capability}"))
    }

    fn build_pipeline_transcode(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<String, SimpleError> {
        if Pipeline::is_webrtcsink(&video_and_stream_information) {
            return Ok(concat!(
                // WebRTCSink requires a raw sink, so whatever is the source's
                // encode, we need to decode it. "decodebin3" does an automatic
                // discovery that works here, so we are simplifying by using it
                // instead of using specific decoders.
                " ! decodebin3",
                " ! videoconvert",
            )
            .to_string());
        }

        let configuration =
            Pipeline::get_video_capture_configuration(video_and_stream_information)?;

        let pipeline_transcode = match &video_and_stream_information.video_source {
            VideoSourceType::Gst(_) => match configuration.encode {
                // Fake sources are video/x-raw, so we need to encode it to
                // have h264 or mjpg.
                VideoEncodeType::H264 => concat!(
                    " ! videoconvert",
                    " ! x264enc bitrate=5000",
                    " ! video/x-h264,profile=baseline",
                ),
                _ => "",
            },
            video_source_type => {
                return Err(SimpleError::new(format!(
                    "Unsupported VideoSourceType: {video_source_type:#?}.",
                )));
            }
        };
        Ok(pipeline_transcode.to_string())
    }

    fn build_pipeline_payload(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<String, SimpleError> {
        if Pipeline::is_webrtcsink(&video_and_stream_information) {
            // WebRTCSink requires no payload.
            return Ok("".to_string());
        }

        let configuration =
            Pipeline::get_video_capture_configuration(&video_and_stream_information)?;

        let pipeline_payload = match &configuration.encode {
            // Here we are naming the payloader as pay0 because the rtsp server
            // expects this specific name, and having a name doesn't hurt any
            // other endpoint type.
            VideoEncodeType::H264 => concat!(
                " ! h264parse",
                " ! queue",
                " ! rtph264pay name=pay0 config-interval=10 pt=96",
            ),
            video_encode_type => {
                return Err(SimpleError::new(format!(
                    "Unsupported VideoEncodeType: {video_encode_type:#?}"
                )))
            }
        };
        Ok(pipeline_payload.to_string())
    }

    fn build_pipeline_sink(
        endpoints: &Vec<url::Url>,
        is_webrtcsink: bool,
    ) -> Result<String, SimpleError> {
        if Pipeline::is_webrtcsink(&video_and_stream_information) {
            let (stun_endpoint, turn_endpoint, signalling_endpoint) =
                Pipeline::build_webrtc_endpoints(&video_and_stream_information)?;
            let capability = "video/x-h264"; // We could also choose for video/x-vp9 here.
                                             // WebRTCSink's congestion control
            return Ok(format!(
                " ! webrtcsink stun-server={stun_endpoint} \
                    turn-server={turn_endpoint} \
                    signaller::address={signalling_endpoint} \
                    video-caps={capability}"
            ));
        }
        let endpoints = &video_and_stream_information.stream_information.endpoints;
        let pipeline_sink = match endpoints[0].scheme() {
            "udp" => {
                let clients = endpoints
                    .iter()
                    .map(|endpoint| {
                        format!("{}:{}", endpoint.host().unwrap(), endpoint.port().unwrap())
                    })
                    .collect::<Vec<String>>()
                    .join(",");
                format!(" ! multiudpsink clients={clients}")
            }
            _ => "".to_string(),
        };
        Ok(pipeline_sink)
    }

    fn build_webrtc_endpoints(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<(url::Url, url::Url, url::Url), SimpleError> {
        let endpoints = &video_and_stream_information.stream_information.endpoints;
        let mut stun_endpoint = url::Url::parse(DEFAULT_STUN_ENDPOINT).unwrap();
        let mut turn_endpoint = url::Url::parse(DEFAULT_TURN_ENDPOINT).unwrap();
        let mut signalling_endpoint = url::Url::parse(DEFAULT_SIGNALLING_ENDPOINT).unwrap();
        for endpoint in endpoints.iter() {
            match endpoint.scheme() {
                "webrtc" => (),
                "stun" => stun_endpoint = endpoint.to_owned(),
                "turn" => turn_endpoint = endpoint.to_owned(),
                "ws" => signalling_endpoint = endpoint.to_owned(),
                _ => {
                    return Err(SimpleError::new(format!(
                        "Only 'webrtc://', 'stun://', 'turn://' and 'ws://' schemes are accepted. {usage_hint}. The scheme passed was: {scheme:#?}\"",
                        usage_hint=webrtc_usage_hint(),
                        scheme=endpoint.scheme()
                    )))
                }
            }
        }
        debug!("Using the following endpoint for the STUN Server: \"{stun_endpoint}\"");
        debug!("Using the following endpoint for the TURN Server: \"{turn_endpoint}\"",);
        debug!("Using the following endpoint for the Signalling Server: \"{signalling_endpoint}\"",);
        Ok((stun_endpoint, turn_endpoint, signalling_endpoint))
    }

    pub fn is_webrtcsink(video_and_stream_information: &VideoAndStreamInformation) -> bool {
        // TODO: Move "webrtc", "stun", "turn", "ws", "udp", "rtsp", "tcp" and other schemes to an enum
        let endpoints = &video_and_stream_information.stream_information.endpoints;
        let mut is_webrtcsink = false;
        for endpoint in endpoints.iter() {
            if matches!(endpoint.scheme(), "webrtc" | "stun" | "turn" | "ws") {
                is_webrtcsink = true;
            }
        }
        is_webrtcsink
    }

    fn get_video_capture_configuration(
        video_and_stream_information: &VideoAndStreamInformation,
    ) -> Result<&VideoCaptureConfiguration, SimpleError> {
        let configuration = match &video_and_stream_information
            .stream_information
            .configuration
        {
            crate::stream::types::CaptureConfiguration::VIDEO(configuration) => configuration,
            crate::stream::types::CaptureConfiguration::REDIRECT(_) => {
                return Err(SimpleError::new(
                    "Error: Cannot create a pipeline from a REDIRECT source!",
                ))
            }
        };
        Ok(configuration)
    }
}
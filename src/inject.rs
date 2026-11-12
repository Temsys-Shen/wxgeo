use crate::config::Config;

pub fn build(cfg: &Config) -> String {
    let tpl = r#"<script>(function(){
var LAT=__LAT__,LON=__LON__,ACC=__ACC__;
function mk(){return {coords:{latitude:LAT,longitude:LON,accuracy:ACC,altitude:null,altitudeAccuracy:null,heading:null,speed:null},timestamp:Date.now()};}
try{if(navigator.geolocation){navigator.geolocation.getCurrentPosition=function(s,e,o){if(s)setTimeout(function(){s(mk());},30);};navigator.geolocation.watchPosition=function(s,e,o){if(s)s(mk());return 1;};navigator.geolocation.clearWatch=function(){};}}catch(e){}
function pw(w){if(!w)return;try{if(w.getLocation)w.getLocation=function(o){o=o||{};var r={latitude:LAT,longitude:LON,accuracy:ACC,speed:0,altitude:0};if(o.success)o.success(r);if(o.complete)o.complete(r);};}catch(e){}}
var _w=setInterval(function(){try{if(window.wx){pw(window.wx);clearInterval(_w);}}catch(e){}},100);setTimeout(function(){clearInterval(_w);},10000);
try{__EXTRA__}catch(e){}
})();</script>"#;
    tpl.replace("__LAT__", &format!("{}", cfg.lat))
        .replace("__LON__", &format!("{}", cfg.lon))
        .replace("__ACC__", &format!("{}", cfg.acc))
        .replace("__EXTRA__", cfg.extra_js.as_str())
}

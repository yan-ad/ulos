#include "utf8_main.hpp"
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_handle.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/version.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/write_resume_data.hpp>
#include <nlohmann/json.hpp>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <chrono>
#include <map>
#ifndef _WIN32
#include <poll.h>
#include <unistd.h>
#endif
#include <cerrno>
#include <iomanip>
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>
#include <sstream>

namespace lt=libtorrent;
namespace fs=std::filesystem;
using json=nlohmann::json;
using Clock=std::chrono::steady_clock;
static void emit(json const& j) { std::cout << j.dump() << '\n' << std::flush; }
static int fail() { emit({{"event","error"},{"message","Torrent worker operation failed"}}); return 1; }
static std::string hex(char const* p, int n) { std::ostringstream s; s << std::hex << std::setfill('0'); for(int i=0;i<n;++i)s<<std::setw(2)<<unsigned(static_cast<unsigned char>(p[i])); return s.str(); }
static unsigned number(std::string const& v, unsigned max) { if(v.empty()||v.size()>10)throw 1; uint64_t n=0; for(char c:v){if(c<'0'||c>'9')throw 1;n=n*10+c-'0';if(n>max)throw 1;}return unsigned(n); }
// One main-thread reader, bounded memory and work even under a continuous flood.
// Windows stdin must be a real redirected pipe, not a console or blocking thread.
class Input {
 std::string pending;
 int available(int wait) {
#ifdef _WIN32
  HANDLE pipe=GetStdHandle(STD_INPUT_HANDLE);
  auto deadline=Clock::now()+std::chrono::milliseconds(wait);
  for(;;){
   DWORD bytes=0;
   if(pipe==nullptr||pipe==INVALID_HANDLE_VALUE){eof=true;return 0;}
   if(!PeekNamedPipe(pipe,nullptr,0,nullptr,&bytes,nullptr)){
    auto error=GetLastError();
    if(error==ERROR_BROKEN_PIPE||error==ERROR_INVALID_HANDLE){eof=true;return 0;}
    throw 1;
   }
   if(bytes)return int(std::min<DWORD>(bytes,1024));
   if(Clock::now()>=deadline)return 0;
   Sleep(10);
  }
#else
  pollfd p{STDIN_FILENO,POLLIN,0};int r;
  do{r=::poll(&p,1,wait);}while(r<0&&errno==EINTR);
  if(r<0)throw 1;
  if(r&&p.revents&POLLNVAL){eof=true;return 0;}
  return r?1024:0;
#endif
 }
public:
 bool eof=false;
 std::vector<json> read(int wait) {
  std::vector<json> commands;size_t consumed=0;
  while(!eof&&consumed<16384){
   int count=available(consumed?0:wait);if(!count)break;
   char b[1024];int n=0;
#ifdef _WIN32
   DWORD got=0;
   if(!ReadFile(GetStdHandle(STD_INPUT_HANDLE),b,DWORD(count),&got,nullptr)){
    auto error=GetLastError();
    if(error==ERROR_BROKEN_PIPE||error==ERROR_INVALID_HANDLE){eof=true;break;}
    throw 1;
   }
   n=int(got);
#else
   auto got=::read(STDIN_FILENO,b,size_t(count));
   if(got<0){if(errno==EINTR)continue;throw 1;}n=int(got);
#endif
   if(!n){eof=true;break;}consumed+=size_t(n);
   for(int i=0;i<n;++i){
    if(b[i]=='\n'){
     if(pending.empty()||commands.size()>=64)throw 1;
     auto j=json::parse(pending);if(!j.is_object())throw 1;
     commands.push_back(std::move(j));pending.clear();
    }else{if(pending.size()>=1024)throw 1;pending+=b[i];}
   }
  }
  if(eof&&!pending.empty())throw 1;
  return commands;
 }
};
static unsigned limit(json const& j,char const* name) {auto const& v=j.at(name);if(!v.is_number_unsigned()&&!v.is_number_integer())throw 1;auto n=v.get<int64_t>();if(n<0||n>1073741824)throw 1;return unsigned(n);}
static void keys(json const& j,std::initializer_list<std::string> allowed){for(auto i=j.begin();i!=j.end();++i){bool ok=false;for(auto const& k:allowed)if(i.key()==k)ok=true;if(!ok)throw 1;}}
// Explicit metadata-only operation. No downloaded payload is ever enabled, even
// during the transition from an unknown magnet to a known torrent.
static int resolve_magnet(std::vector<std::string> const& argv) {
 std::map<std::string,std::string> a;
 if(argv.size()!=9)throw 1;
 for(size_t i=1;i<argv.size();i+=2){
  auto const& k=argv[i];auto const& v=argv[i+1];
  if(k!="--resolve-magnet"&&k!="--output"&&k!="--dht"&&k!="--timeout")throw 1;
  if(v.size()>(k=="--resolve-magnet"?16384u:4096u)||!a.emplace(k,v).second)throw 1;
 }
 auto timeout=number(a.at("--timeout"),120);if(!timeout)throw 1;
 bool dht=number(a.at("--dht"),1);
 auto const& uri=a.at("--resolve-magnet");if(uri.rfind("magnet:?",0)!=0)throw 1;
 fs::path out=fs::u8path(a.at("--output"));
 if(!out.is_absolute()||!fs::is_directory(fs::symlink_status(out))||!fs::is_empty(out))throw 1;
 // libtorrent 2.1 parses literal x.pe endpoints but does not unescape their
 // values. Normalize this one numeric endpoint field only (never tracker URLs).
 std::string normalized="magnet:?";size_t pos=8;
 while(pos<=uri.size()){
  auto end=uri.find('&',pos);if(end==std::string::npos)end=uri.size();
  auto field=uri.substr(pos,end-pos);
  if(field.rfind("x.pe=",0)==0){
   std::string endpoint;
   for(size_t i=5;i<field.size();++i){
    char c=field[i];
    if(c=='%'){
     if(i+2>=field.size())throw 1;
     auto digit=[](char v)->int{if(v>='0'&&v<='9')return v-'0';if(v>='a'&&v<='f')return v-'a'+10;if(v>='A'&&v<='F')return v-'A'+10;throw 1;};
     c=char(digit(field[i+1])*16+digit(field[i+2]));i+=2;
    }
    if(!((c>='0'&&c<='9')||(c>='a'&&c<='f')||(c>='A'&&c<='F')||c==':'||c=='.'||c=='['||c==']'))throw 1;
    endpoint+=c;
   }
   auto colon=endpoint.rfind(':');if(colon==std::string::npos)throw 1;
   auto host=endpoint.substr(0,colon);if(host.size()>2&&host.front()=='['&&host.back()==']')host=host.substr(1,host.size()-2);
   lt::error_code address_error;lt::make_address(host,address_error);
   if(address_error||!number(endpoint.substr(colon+1),65535))throw 1;
   field="x.pe="+endpoint;
  }
  normalized+=field;if(end==uri.size())break;normalized+='&';pos=end+1;
 }
 lt::error_code ec;auto p=lt::parse_magnet_uri(normalized,ec);if(ec||(!p.info_hashes.has_v1()&&!p.info_hashes.has_v2()))throw 1;
 // Reject web seeds, rather than contacting or silently trusting URI endpoints.
 // Tracker credential and endpoint policy validation also occurs in the parent.
 if(!p.url_seeds.empty()||!p.http_seeds.empty())throw 1;
 p.url_seeds.clear();p.http_seeds.clear();p.save_path=out.u8string();
 p.flags &= ~(lt::torrent_flags::auto_managed|lt::torrent_flags::paused);
 p.flags |= lt::torrent_flags::upload_mode|lt::torrent_flags::disable_lsd;
 // stop_when_ready would stop *before* BEP 9 metadata exchange (libtorrent
 // explicitly counts metadata exchange as downloading). Use upload_mode plus
 // zero priorities instead; never clear upload_mode or resume payload downloads.
 p.file_priorities.assign(4096,lt::dont_download);
 emit({{"event","ready"},{"protocol",1},{"engine","libtorrent"},{"version",LIBTORRENT_VERSION}});
 Input input;bool cancelled=false;std::vector<char> metainfo;
 auto deadline=Clock::now()+std::chrono::seconds(timeout);
 {
  lt::settings_pack s;s.set_bool(lt::settings_pack::enable_dht,dht);
  s.set_bool(lt::settings_pack::enable_lsd,false);s.set_bool(lt::settings_pack::enable_upnp,false);s.set_bool(lt::settings_pack::enable_natpmp,false);
  s.set_str(lt::settings_pack::listen_interfaces,"0.0.0.0:0");
  s.set_int(lt::settings_pack::max_metadata_size,8*1024*1024);
  s.set_int(lt::settings_pack::connections_limit,50);
  s.set_int(lt::settings_pack::alert_mask,lt::alert_category::error|lt::alert_category::storage|lt::alert_category::status);
  lt::session session(s);auto h=session.add_torrent(p,ec);if(ec)throw 1;
  bool saving=false;
  while(metainfo.empty()&&!cancelled){
   auto commands=input.read(100);
   for(auto const& c:commands){keys(c,{"command"});if(c.at("command")!="cancel")throw 1;cancelled=true;}
   if(input.eof)cancelled=true;
   if(cancelled)break;
   if(Clock::now()>=deadline)throw 1;
   std::vector<lt::alert*> alerts;session.pop_alerts(&alerts);
   for(auto* alert:alerts){
    if(lt::alert_cast<lt::torrent_error_alert>(alert)||lt::alert_cast<lt::save_resume_data_failed_alert>(alert))throw 1;
    if(auto* saved=lt::alert_cast<lt::save_resume_data_alert>(alert)){
     // Default flags deliberately REQUIRE complete BEP 52 piece layers. BEP 9
     // supplies the info dictionary only: never synthesize missing hash proofs.
     saved->params.url_seeds.clear();saved->params.http_seeds.clear();
     metainfo=lt::write_torrent_file_buf(saved->params,lt::write_flags::no_http_seeds);
     if(metainfo.empty()||metainfo.size()>8*1024*1024)throw 1;
    }
   }
   if(!saving&&h.status().has_metadata){
    auto ti=h.torrent_file();if(!ti||ti->metadata_size()>8*1024*1024)throw 1;
    if(ti->priv()){
     h.set_flags(lt::torrent_flags::disable_dht|lt::torrent_flags::disable_pex);
     lt::settings_pack off;off.set_bool(lt::settings_pack::enable_dht,false);session.apply_settings(off);
    }
    h.pause();h.save_resume_data(lt::torrent_handle::save_info_dict);saving=true;
   }
  }
  h.pause();
  // Destruction joins disk/network threads before any terminal event is emitted.
 }
 if(!fs::is_empty(out))throw 1;
 if(cancelled)emit({{"event","cancelled"}});
 else emit({{"event","metadata"},{"metainfo_hex",hex(metainfo.data(),int(metainfo.size()))}});
 return 0;
}
static int worker_main(std::vector<std::string> const& argv) {
 int argc=int(argv.size());
 try {
  if(LIBTORRENT_VERSION_MAJOR!=2||argc>19)throw 1;
  if(std::find(argv.begin()+1,argv.end(),"--resolve-magnet")!=argv.end())return resolve_magnet(argv);
  std::map<std::string,std::string> a;
  for(int i=1;i<argc;i+=2){if(i+1>=argc||std::string(argv[i+1]).size()>4096)throw 1;std::string k=argv[i];if(k!="--metainfo"&&k!="--output"&&k!="--dht"&&k!="--upload"&&k!="--download-limit"&&k!="--upload-limit"&&k!="--peer-limit")throw 1;if(!a.emplace(k,argv[i+1]).second)throw 1;}
  if(a.size()!=7)throw 1;
  bool dht=number(a.at("--dht"),1),upload=number(a.at("--upload"),1);
  unsigned down=number(a.at("--download-limit"),1073741824),up=number(a.at("--upload-limit"),1073741824),peers=number(a.at("--peer-limit"),10000);
  if(!peers)throw 1;
  fs::path meta=fs::u8path(a.at("--metainfo")),out=fs::u8path(a.at("--output"));
  if(!meta.is_absolute()||!out.is_absolute()||!fs::is_regular_file(fs::symlink_status(meta))||fs::file_size(meta)>8*1024*1024||!fs::is_directory(fs::symlink_status(out))||!fs::is_empty(out))throw 1;
  std::ifstream f(meta,std::ios::binary);std::vector<char> bytes(8*1024*1024+1);f.read(bytes.data(),std::streamsize(bytes.size()));auto count=f.gcount();if(f.bad()||count<=0||count>8*1024*1024)throw 1;bytes.resize(size_t(count));
  lt::error_code ec;auto ti=std::make_shared<lt::torrent_info>(bytes.data(),int(bytes.size()),ec);if(ec)throw 1;
  // Metainfo URL seeds are not part of the approved tracker/peer policy.
  // The Rust parser rejects them; preserve the boundary if this helper is used
  // directly rather than silently contacting unreviewed HTTP endpoints.
  if(!ti->web_seeds().empty())throw 1;
  json files=json::array();uint64_t total=0;
  for(auto idx:ti->files().file_range()){
   auto flags=ti->files().file_flags(idx);if(flags & lt::file_storage::flag_symlink)throw 1;
   fs::path p=fs::u8path(ti->files().file_path(idx));if(p.empty()||p.is_absolute())throw 1;for(auto const& c:p)if(c==".."||c==".")throw 1;
   if(flags & lt::file_storage::flag_pad_file)continue;
   auto size=ti->files().file_size(idx);if(size<0)throw 1;total+=uint64_t(size);files.push_back({{"path",p.generic_u8string()},{"length",size}});
  }
  auto hashes=ti->info_hashes();json manifest={{"event","manifest"},{"files",files},{"name",ti->name()},{"private",ti->priv()}};
  if(hashes.has_v1())manifest["infohash_v1"]=hex(hashes.v1.data(),20);
  if(hashes.has_v2())manifest["infohash_v2"]=hex(hashes.v2.data(),32);
  emit({{"event","ready"},{"protocol",1},{"engine","libtorrent"},{"version",LIBTORRENT_VERSION},{"capabilities",{{"no_upload",false}}}});emit(manifest);
  Input input;bool start=false;std::vector<json> queued;std::vector<lt::tcp::endpoint> initial_peers;
  while(!start){auto commands=input.read(100);if(input.eof){emit({{"event","cancelled"}});return 0;}for(auto const& c:commands){if(start){if(queued.size()>=64)throw 1;queued.push_back(c);continue;}if(c.at("command")=="peer"){keys(c,{"command","address","port"});auto ip=lt::make_address(c.at("address").get<std::string>(),ec);auto port=limit(c,"port");if(ec||ip.is_unspecified()||ip.is_multicast()||port==0||port>65535||initial_peers.size()>=64)throw 1;initial_peers.emplace_back(ip,static_cast<unsigned short>(port));continue;}keys(c,{"command"});if(c.at("command")!="start")throw 1;start=true;}}
  // libtorrent has no supported hard no-upload switch. Fail closed rather than
  // interpreting upload_mode (which disables downloads) or a zero rate (unlimited).
  if(!upload){emit({{"event","error"},{"message","This engine cannot guarantee no-upload mode"}});return 1;}
  bool complete=false,cancel=false;
  {
   lt::settings_pack s;s.set_bool(lt::settings_pack::enable_dht,dht&&!ti->priv());s.set_bool(lt::settings_pack::enable_lsd,false);s.set_bool(lt::settings_pack::enable_upnp,false);s.set_bool(lt::settings_pack::enable_natpmp,false);
   s.set_str(lt::settings_pack::listen_interfaces,"0.0.0.0:0");
   // Session limits would remain a hidden ceiling after live per-torrent
   // updates. This isolated session has exactly one torrent: apply limits there.
   s.set_int(lt::settings_pack::connections_limit,10000);
   s.set_int(lt::settings_pack::alert_mask,lt::alert_category::error|lt::alert_category::status);
   lt::session session(s);
   lt::add_torrent_params p;p.ti=ti;p.save_path=out.u8string();p.flags &= ~lt::torrent_flags::auto_managed;p.flags &= ~lt::torrent_flags::paused;
   p.peers=std::move(initial_peers);p.download_limit=int(down);p.upload_limit=int(up);p.max_connections=int(peers);
   p.flags |= lt::torrent_flags::disable_lsd;
   if(ti->priv())p.flags |= lt::torrent_flags::disable_dht | lt::torrent_flags::disable_pex;
   auto h=session.add_torrent(p,ec);if(ec)throw 1;
   auto last=Clock::now();int64_t previous=-1;std::string previous_phase;bool pause_requested=false,pause_ack=false;
   while(!cancel&&!complete){
    auto commands=std::move(queued);queued.clear();if(commands.empty())commands=input.read(100);
    for(auto const& c:commands){
     auto cmd=c.at("command").get<std::string>();
     if(cmd=="limits"){keys(c,{"command","download_bytes_per_second","upload_bytes_per_second"});h.set_download_limit(int(limit(c,"download_bytes_per_second")));h.set_upload_limit(int(limit(c,"upload_bytes_per_second")));}
     else if(cmd=="peers"){keys(c,{"command","limit"});auto n=limit(c,"limit");if(n==0||n>10000)throw 1;h.set_max_connections(int(n));}
      else if(cmd=="connect_peer"){
      keys(c,{"command","address","port"});auto ip=lt::make_address(c.at("address").get<std::string>(),ec);auto port=limit(c,"port");if(ec||!ip.is_loopback()||port==0||port>65535)throw 1;h.connect_peer(lt::tcp::endpoint(ip,static_cast<unsigned short>(port)));
     }else{keys(c,{"command"});if(cmd=="pause"){if(!pause_requested){pause_requested=true;pause_ack=false;h.pause();}}else if(cmd=="resume"){if(pause_requested){pause_requested=false;pause_ack=false;h.resume();}}else if(cmd=="cancel")cancel=true;else throw 1;}
    }
    if(input.eof)cancel=true;
    std::vector<lt::alert*> alerts;session.pop_alerts(&alerts);for(auto* alert:alerts){if(lt::alert_cast<lt::torrent_error_alert>(alert))throw 1;if(lt::alert_cast<lt::torrent_paused_alert>(alert)&&pause_requested)pause_ack=true;}
    if(cancel)break;
    auto st=h.status();if(st.errc)throw 1;
    auto now=Clock::now();auto progress=st.total_payload_download+st.total_done;
    bool paused=pause_requested&&pause_ack&&bool(st.flags & lt::torrent_flags::paused);
    bool checking=st.state==lt::torrent_status::checking_files||st.state==lt::torrent_status::checking_resume_data;
    complete=!pause_requested&&!checking&&st.is_finished&&uint64_t(st.total_wanted_done)==total;
     std::string phase=complete?"publishing":paused?"paused":checking?"checking":"downloading";
     if(progress!=previous||phase!=previous_phase||phase!="downloading")last=now;
     previous=progress;previous_phase=phase;
    // num_seeds is the exact connected-seed count. distributed_copies is the
    // engine's fractional availability metric; -1 means libtorrent does not
    // track it (notably when seeding), so preserve that as JSON null.
    json availability=nullptr;
    if(phase=="downloading"&&std::isfinite(st.distributed_copies)&&st.distributed_copies>=0.f&&static_cast<double>(st.distributed_copies)<=std::numeric_limits<uint32_t>::max()) availability=st.distributed_copies;
    emit({{"event","stats"},{"phase",phase},{"downloaded_bytes",st.total_wanted_done},{"total_bytes",total},{"download_bytes_per_second",paused?0:st.download_payload_rate},{"upload_bytes_per_second",paused?0:st.upload_payload_rate},{"payload_downloaded_bytes",std::max(int64_t(0),st.all_time_download)},{"uploaded_bytes",std::max(int64_t(0),st.all_time_upload)},{"peers",st.num_peers},{"seeds",phase=="downloading"&&st.num_seeds>=0?json(st.num_seeds):json(nullptr)},{"availability",availability},{"connecting_peers",std::max(0,st.num_connections-st.num_peers)},{"queued_peers",st.connect_candidates},{"seen_peers",st.list_peers},{"stalled_for_secs",std::chrono::duration_cast<std::chrono::seconds>(now-last).count()},{"tracker_count",ti->trackers().size()}});
   }
   h.pause();
   // The session destructor synchronously joins its network/disk workers. No
   // complete/cancelled event is sent while its file handles remain open.
  }
  emit({{"event",complete?"complete":"cancelled"}});return 0;
 }catch(...){return fail();}
}

#ifdef _WIN32
int wmain(int argc,wchar_t** argv) {
#else
int main(int argc,char** argv) {
#endif
 try{return worker_main(utf8_arguments(argc,argv));}catch(...){return fail();}
}

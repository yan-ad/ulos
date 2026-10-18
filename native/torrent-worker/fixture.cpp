#include "utf8_main.hpp"
// Local acceptance-test utility, not installed or shipped with the application.
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_handle.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <nlohmann/json.hpp>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>
namespace lt=libtorrent;
namespace fs=std::filesystem;
// Match the worker's bounded UTF-8 filesystem/buffer boundary. Do not delegate
// opening a test path to libtorrent's deprecated filename constructor.
static std::shared_ptr<lt::torrent_info> read_torrent(std::string const& name){
 fs::path path=fs::u8path(name);if(!fs::is_regular_file(fs::symlink_status(path))||fs::file_size(path)>8*1024*1024)throw 1;
 std::ifstream input(path,std::ios::binary);std::vector<char> bytes(8*1024*1024+1);input.read(bytes.data(),std::streamsize(bytes.size()));auto count=input.gcount();if(input.bad()||count<=0||count>8*1024*1024)throw 1;bytes.resize(size_t(count));
 lt::error_code ec;auto result=std::make_shared<lt::torrent_info>(bytes.data(),int(bytes.size()),ec);if(ec)throw 1;return result;
}
static int fixture_main(std::vector<std::string> const& argv){int argc=int(argv.size());try{
 if(argc==3&&argv[1]=="--magnet-info"){
  lt::add_torrent_params p;p.ti=read_torrent(argv[2]);
  std::cout<<nlohmann::json({{"magnet",lt::make_magnet_uri(p)}}).dump()<<std::endl;return 0;
 }
 if((argc==6||argc==8)&&std::string(argv[1])=="--create-fixture"&&std::string(argv[3])=="--torrent-out"){
  std::filesystem::path data=std::filesystem::absolute(std::filesystem::u8path(argv[2]));lt::file_storage files;lt::add_files(files,data.u8string());
  auto mode=std::string(argv[5]);auto flags=lt::create_torrent::v2_only;if(mode=="--hybrid")flags={};else if(mode=="--v1")flags=lt::create_torrent::v1_only;else if(mode!="--v2")throw 1;
  lt::create_torrent t(files,16384,flags);
  // Test-only tracker injection enables the parent integration test to discover
  // its localhost seeder normally, without a production connect_peer command.
  if(argc==8){if(argv[6]!="--tracker"||argv[7].empty())throw 1;t.add_tracker(argv[7]);}
  lt::error_code ec;lt::set_piece_hashes(t,data.parent_path().u8string(),ec);if(ec)throw 1;
  std::vector<char> encoded;lt::bencode(std::back_inserter(encoded),t.generate());std::ofstream o(std::filesystem::u8path(argv[4]),std::ios::binary);o.write(encoded.data(),encoded.size());if(!o)throw 1;return 0;
 }
 if(argc!=7||std::string(argv[1])!="--seed-test"||std::string(argv[3])!="--data"||std::string(argv[5])!="--listen")throw 1;
 std::string listen=argv[6];if(listen.rfind("127.0.0.1:",0)!=0)throw 1;
 lt::settings_pack s;s.set_str(lt::settings_pack::listen_interfaces,listen);s.set_bool(lt::settings_pack::enable_dht,false);s.set_bool(lt::settings_pack::enable_lsd,false);s.set_bool(lt::settings_pack::enable_upnp,false);s.set_bool(lt::settings_pack::enable_natpmp,false);
 lt::add_torrent_params p;p.ti=read_torrent(argv[2]);p.save_path=argv[4];p.flags &= ~lt::torrent_flags::paused;p.flags &= ~lt::torrent_flags::auto_managed;
 lt::session session(s);auto h=session.add_torrent(p);
 for(int i=0;i<300;++i){
  auto status=h.status();
  if(status.errc){std::cerr<<"Fixture seed check failed; error="<<status.errc.value()<<"\n";throw 1;}
  if(status.is_seeding&&session.listen_port()!=0)break;
  std::this_thread::sleep_for(std::chrono::milliseconds(100));
 }
 if(!h.status().is_seeding||session.listen_port()==0){
  auto status=h.status();std::cerr<<"Fixture seed readiness failed; state="<<int(status.state)<<"; seeding="<<status.is_seeding<<"; listen="<<session.listen_port()<<"; checked="<<status.total_done<<"\n";throw 1;
 }
 std::cout<<nlohmann::json({{"event","seed_ready"},{"port",session.listen_port()}}).dump()<<std::endl;
 std::string line;std::getline(std::cin,line);return 0;
}catch(...){std::cerr<<"Fixture operation failed\n";return 1;}}

#ifdef _WIN32
int wmain(int argc,wchar_t** argv) {
#else
int main(int argc,char** argv) {
#endif
 try{return fixture_main(utf8_arguments(argc,argv));}catch(...){std::cerr<<"Fixture operation failed\n";return 1;}
}

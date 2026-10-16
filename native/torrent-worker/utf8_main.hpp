#pragma once
// Define NOMINMAX before any dependency can transitively include Windows headers.
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <Windows.h>
#endif
#include <string>
#include <vector>

// Keep filesystem/libtorrent boundaries UTF-8 even with non-ASCII Windows argv.
// Conversion errors are handled by the caller without printing argument contents.
#ifdef _WIN32
static std::vector<std::string> utf8_arguments(int argc, wchar_t** argv) {
 std::vector<std::string> result;
 for(int i=0;i<argc;++i) {
  int n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,argv[i],-1,nullptr,0,nullptr,nullptr);
  if(n<=0)throw 1;
  std::string s(size_t(n),'\0');
  if(WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,argv[i],-1,s.data(),n,nullptr,nullptr)!=n)throw 1;
  s.pop_back();result.push_back(std::move(s));
 }
 return result;
}
#else
static std::vector<std::string> utf8_arguments(int argc, char** argv) {
 return {argv,argv+argc};
}
#endif

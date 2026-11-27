# Live captions build whisper.cpp through the whisper-rs-sys crate, which links ggml as
# `-l static=ggml` (and ggml-base, ggml-cpu): for x86_64-pc-windows-gnu rustc looks for
# libggml.a. ggml's CMakeLists.txt drops the `lib` prefix on every Windows build, though
# (`set(CMAKE_STATIC_LIBRARY_PREFIX "")` under `if (WIN32)`), so a MinGW build writes ggml.a
# and the link fails ("could not find native static library `ggml`").
#
# build-server.sh hands this file to CMake as CMAKE_PROJECT_ggml_INCLUDE (whisper-rs-sys
# passes CMAKE_* environment variables on to CMake). It runs right after `project(ggml)`, before
# that `set`, so it defers to the end of ggml's directory and puts the prefix back on each
# static library target there: libggml.a, libggml-base.a, libggml-cpu.a.
function(vesper_ggml_lib_prefix dir)
  get_property(targets DIRECTORY "${dir}" PROPERTY BUILDSYSTEM_TARGETS)
  get_property(subdirs DIRECTORY "${dir}" PROPERTY SUBDIRECTORIES)
  foreach(target IN LISTS targets)
    get_target_property(type "${target}" TYPE)
    if(type STREQUAL "STATIC_LIBRARY")
      set_target_properties("${target}" PROPERTIES PREFIX "lib")
    endif()
  endforeach()
  foreach(subdir IN LISTS subdirs)
    vesper_ggml_lib_prefix("${subdir}")
  endforeach()
endfunction()

cmake_language(DEFER CALL vesper_ggml_lib_prefix "${CMAKE_CURRENT_SOURCE_DIR}")

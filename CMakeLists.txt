cmake_minimum_required(VERSION 3.16)
project(NanoAODToolsCpp LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(ROOT REQUIRED COMPONENTS Core MathCore Physics RIO Tree TreePlayer)
enable_testing()

add_library(nanohrt_cpp
  src/core/BranchSchema.cpp
  src/io/NanoReader.cpp
  src/core/Event.cpp
  src/core/Collection.cpp
  src/core/OutputModel.cpp
  src/core/Helpers.cpp
  src/producers/HeavyFlavBaseProducer.cpp
  src/producers/HeavyFlavMuonSampleProducer.cpp
)

target_include_directories(nanohrt_cpp PUBLIC include)
target_link_libraries(nanohrt_cpp PUBLIC ROOT::Core ROOT::MathCore ROOT::Physics ROOT::RIO ROOT::Tree ROOT::TreePlayer)

add_executable(muon_framework app/muon_framework.cpp)
target_link_libraries(muon_framework PRIVATE nanohrt_cpp)

add_executable(muon_smoke_test tests/muon_smoke_test.cpp)
target_link_libraries(muon_smoke_test PRIVATE nanohrt_cpp)

add_test(
  NAME muon_smoke_test
  COMMAND muon_smoke_test /eos/cms/store/user/coli/TTto2L2Nu_nano_example.root Events
)

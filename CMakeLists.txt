cmake_minimum_required(VERSION 3.16)
project(NanoAODToolsCpp LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(ROOT REQUIRED COMPONENTS Core MathCore Physics RIO Tree TreePlayer)
find_package(yaml-cpp REQUIRED)
enable_testing()

set(SKBUILD ON CACHE BOOL "Disable CMSJMECalculators python/dictionary build" FORCE)
add_subdirectory(external/CMSJMECalculators EXCLUDE_FROM_ALL)
unset(SKBUILD CACHE)

add_library(nanoaodtools_cpp
  src/core/BranchSchema.cpp
  src/io/NanoReader.cpp
  src/io/RootOutputFile.cpp
  src/core/Event.cpp
  src/core/Collection.cpp
  src/core/OutputModel.cpp
  src/core/Helpers.cpp
  src/helpers/FatjetGenMatching.cpp
  src/helpers/JetMETCorrector.cpp
  src/helpers/PuWeightProducer.cpp
  src/helpers/TopPtWeightProducer.cpp
  src/producers/HeavyFlavBaseProducer.cpp
  src/producers/HeavyFlavMuonSampleProducer.cpp
)

target_include_directories(nanoaodtools_cpp PUBLIC include)
target_link_libraries(nanoaodtools_cpp PUBLIC CMSJMECalculators ROOT::Core ROOT::MathCore ROOT::Physics ROOT::RIO ROOT::Tree ROOT::TreePlayer yaml-cpp)

add_executable(nano_run app/nano_run.cpp)
target_link_libraries(nano_run PRIVATE nanoaodtools_cpp)

add_executable(nano_make_condor app/nano_make_condor.cpp)
target_link_libraries(nano_make_condor PRIVATE nanoaodtools_cpp)

add_executable(muon_smoke_test tests/muon_smoke_test.cpp)
target_link_libraries(muon_smoke_test PRIVATE nanoaodtools_cpp)
target_include_directories(muon_smoke_test PRIVATE app)
target_compile_definitions(muon_smoke_test PRIVATE NANOAODTOOLS_SOURCE_DIR="${CMAKE_SOURCE_DIR}")

add_test(
  NAME muon_smoke_test
  COMMAND muon_smoke_test ${CMAKE_SOURCE_DIR}/../tmp-data/TTto2L2Nu_nano_2024_example_bf09158c-9766-4221-a400-be4ebad0f8b8.root Events
)
